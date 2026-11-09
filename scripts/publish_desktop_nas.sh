#!/usr/bin/env bash
# Publish verified immutable runtime packages; never start a client or host.
# --macos-signer-current accepts a schema-1 desktop-releases.json with exactly
# one pinned macOS ZIP entry. Its build and signer_sha1 must equal the native
# guard's signed-archive report. The receiver atomically advances only
# desktop-releases-macos-<signer_sha1>.json; generic metadata and index stay put.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
STAGE="${1:?Usage: publish_desktop_nas.sh ARTIFACT_DIRECTORY [--macos-only|--macos-signer-current] [--dry-run|--verify-only]}"
shift
MODE=apply
SCOPE=all
VERIFY_OPTIONS=(--desktop)
for option in "$@"; do
  case "$option" in
    --macos-only) [[ "$SCOPE" == all ]] || exit 2; SCOPE=macos; VERIFY_OPTIONS=(--desktop --macos-only) ;;
    --macos-signer-current) [[ "$SCOPE" == all ]] || exit 2; SCOPE=signer; VERIFY_OPTIONS=(--desktop --macos-only) ;;
    apply|--dry-run|--verify-only) [[ "$MODE" == apply ]] || exit 2; MODE="$option" ;;
    *) echo "Unknown publication option: $option" >&2; exit 2 ;;
  esac
done
# A publisher's mutable staging directory must not be re-read after the native
# verifier authorizes a package. Use one private snapshot for verification and upload.
SNAPSHOT="$(mktemp -d "${TMPDIR:-/tmp}/weavelo-desktop-publish.XXXXXX")"
trap 'rm -rf -- "$SNAPSHOT"' EXIT
SNAPSHOT="$(cd -- "$SNAPSHOT" && pwd -P)"
python3 - "$STAGE" "$SNAPSHOT" "$SCOPE" <<'SNAPSHOT_STAGE'
from pathlib import Path
import json, shutil, sys
source, snapshot = map(Path, sys.argv[1:3])
scope = sys.argv[3]
def copy(name):
 assert Path(name).name == name and name not in ('', '.', '..'), 'invalid snapshot name'
 path = source / name
 assert path.is_file() and not path.is_symlink(), 'missing or linked snapshot source'
 shutil.copyfile(path, snapshot / name)
copy('desktop-releases.json')
doc = json.loads((snapshot / 'desktop-releases.json').read_text())
entries = doc['releases']
for entry in entries:
 if scope == 'all' or 'macos' in entry['file'].lower():
  copy(entry['file'])
if scope != 'signer':
 copy(doc.get('acceptance_file', 'acceptance-2.0.0-alpha.2.html'))
 copy('index.html')
if scope == 'macos':
 copy('desktop-releases.before.json')
 copy('index.before.html')
SNAPSHOT_STAGE
STAGE="$SNAPSHOT"
# The exact macOS signature and source identity are checked before networking.
GUARD_REPORT="$(python3 "$ROOT/scripts/macos_release_guard.py" verify-stage "$STAGE" "${VERIFY_OPTIONS[@]}")"
printf '%s\n' "$GUARD_REPORT"
python3 - "$STAGE" "$SCOPE" "$GUARD_REPORT" <<'PREFLIGHT'
from pathlib import Path
import json,sys
stage=Path(sys.argv[1]);scope=sys.argv[2]
guard=json.loads(sys.argv[3].removeprefix('MACOS_RELEASE_VERIFIED '))
doc=json.loads((stage/'desktop-releases.json').read_text())
entries=doc['releases']
assert len({e['file'] for e in entries})==len(entries),'duplicate runtime filenames'
selected=[e for e in entries if 'macos' in e['file'].lower()] if scope in ('macos','signer') else entries
assert len(selected)==(1 if scope in ('macos','signer') else 3),'unexpected runtime package selection'
for entry in entries:
 name=entry['file'];assert Path(name).name==name and name.endswith(('.zip','.tar.gz','.tgz')),'runtime archives only; no source checkout'
metadata=['desktop-releases.json'] if scope=='signer' else ['desktop-releases.json',doc.get('acceptance_file','acceptance-2.0.0-alpha.2.html'),'index.html']
if scope=='signer':
 assert len(entries)==1 and doc.get('schema')==1,'signer manifest must contain exactly one macOS entry'
 entry=selected[0]; signed=guard[0]
 assert entry.get('signer_sha1')==signed['certificate_sha1'] and entry.get('build')==signed['build'],'signer and build must match verified signed archive'
 assert entry.get('build_identity_expectations'),'macOS update requires verified source identity pins'
 assert entry.get('source_commit')==entry['build_identity_expectations'].get('commit'),'source commit must match verified identity'
 import re
 assert re.fullmatch(r'Weavelo-desktop-macos-arm64-[A-Za-z0-9._-]+\.zip', entry['file']),'invalid macOS runtime package name'
if scope=='macos':
 metadata+=['desktop-releases.before.json','index.before.html']
 before=json.loads((stage/'desktop-releases.before.json').read_text())
 assert [e for e in entries if 'macos' not in e['file'].lower()]==[e for e in before['releases'] if 'macos' not in e['file'].lower()],'other platform entries changed'
 assert len([e for e in before['releases'] if 'macos' in e['file'].lower()])==1,'missing original macOS release'
 assert selected[0].get('build_identity_expectations'),'macOS update requires verified source identity pins'
 assert doc['acceptance_file']!=before['acceptance_file'],'preserve the previous acceptance page'
 assert {k:v for k,v in doc.items() if k not in ('releases','published_at','acceptance_file')}=={k:v for k,v in before.items() if k not in ('releases','published_at','acceptance_file')},'unrelated publication metadata changed'
 assert (stage/'index.html').read_bytes()==(stage/'index.before.html').read_bytes().replace(before['acceptance_file'].encode(),doc['acceptance_file'].encode()),'index must preserve unrelated content'
for name in metadata:
 assert Path(name).name==name and (stage/name).is_file() and not (stage/name).is_symlink(),'missing/linked publication metadata'
PREFLIGHT

NAS_HOST="${WEAVELO_NAS_HOST:-root@hackerlife.fun}"
NAS_PORT="${WEAVELO_NAS_PORT:-222}"
REMOTE_DIR="${WEAVELO_NAS_DOWNLOAD_DIR:-/opt/weavelo/downloads}"
BASE="${WEAVELO_DOWNLOAD_BASE_URL:-https://weavelo.hackerlife.fun:8443/download}"
if [[ "$MODE" == --dry-run ]]; then
  python3 - "$STAGE" "$SCOPE" <<'PLAN'
import hashlib,json,sys
from pathlib import Path
stage=Path(sys.argv[1]);scope=sys.argv[2];doc=json.loads((stage/'desktop-releases.json').read_text())
selected=[e for e in doc['releases'] if scope=='all' or 'macos' in e['file'].lower()]
print(json.dumps({'step':'publish','mode':'dry-run','scope':scope,'NAS_role':'runtime-artifact-receiver-only','NAS_compilation':False,'releases':[{k:e[k] for k in ['file','version','sha256','size_bytes']} for e in selected],'preserved_releases':[e['file'] for e in doc['releases'] if e not in selected],'base_manifest_sha256':hashlib.sha256((stage/'desktop-releases.before.json').read_bytes()).hexdigest() if scope=='macos' else None}))
PLAN
  exit 0
fi

ARCHIVE="$(mktemp "$SNAPSHOT/rp-desktop.XXXXXX")"
VERIFY="$(mktemp "$SNAPSHOT/rp-download.XXXXXX")"
python3 - "$STAGE" "$ARCHIVE" "$SCOPE" "$GUARD_REPORT" <<'PACK_VERIFIED_STAGE'
from pathlib import Path
import hashlib,io,json,sys,tarfile
stage=Path(sys.argv[1]);doc=json.loads((stage/'desktop-releases.json').read_text());scope=sys.argv[3]
entries=[e for e in doc['releases'] if scope=='all' or 'macos' in e['file'].lower()]
guard=json.loads(sys.argv[4].removeprefix('MACOS_RELEASE_VERIFIED '))[0]
with tarfile.open(sys.argv[2],'w') as archive:
 for entry in entries:
  name=entry['file'];assert Path(name).name==name
  p=stage/name;assert p.is_file() and not p.is_symlink()
  with p.open('rb') as source:
   data=source.read()
  actual=hashlib.sha256(data).hexdigest()
  assert len(data)==entry['size_bytes'] and actual==entry['sha256']
  if 'macos' in name.lower():
   assert actual==guard['archive_sha256'] and len(data)==guard['size_bytes'],'upload bytes differ from native guard'
  member=tarfile.TarInfo(name)
  member.size=len(data);member.mode=0o644
  archive.addfile(member,io.BytesIO(data))
 names=['desktop-releases.json'] if scope=='signer' else ['desktop-releases.json',doc.get('acceptance_file','acceptance-2.0.0-alpha.2.html'),'index.html']
 if scope=='macos':names+=['desktop-releases.before.json','index.before.html']
 if scope!='signer':
  mac=next(e for e in entries if 'macos' in e['file'].lower())
  if mac.get('build_identity_expectations'):
   pinned=dict(mac,signer_sha1=guard['certificate_sha1'],build=guard['build'])
   (stage/'macos-signer-current.json').write_text(json.dumps({'schema':1,'releases':[pinned]})+'\n')
   names+=['macos-signer-current.json']
 for name in names:
  assert Path(name).name==name
  archive.add(stage/name,arcname=name,recursive=False)
PACK_VERIFIED_STAGE
REMOTE_ARCHIVE="/tmp/weavelo-desktop-$(basename "$ARCHIVE")"
if [[ "$MODE" != --verify-only ]]; then
scp -O -P "$NAS_PORT" -o BatchMode=yes -o ConnectTimeout=10 "$ARCHIVE" "$NAS_HOST:$REMOTE_ARCHIVE"
ssh -p "$NAS_PORT" -o BatchMode=yes -o ConnectTimeout=10 "$NAS_HOST" "python3 - '$REMOTE_ARCHIVE' '$REMOTE_DIR' '$SCOPE'" <<'PY'
from pathlib import Path
import fcntl,hashlib,json,os,re,shutil,sys,tarfile,tempfile
archive=Path(sys.argv[1]);root=Path(sys.argv[2]);scope=sys.argv[3] if len(sys.argv)>3 else 'all'
assert scope in ('all','macos','signer')
assert not root.is_symlink(),'linked publication root'
if scope in ('macos','signer'):assert root.is_dir(),'existing publication required'
else:root.mkdir(parents=True,exist_ok=True)
def digest(path):
 h=hashlib.sha256()
 with path.open('rb') as stream:
  for block in iter(lambda:stream.read(1024*1024),b''):h.update(block)
 return h.hexdigest()
def regular(path):
 assert path.is_file() and not path.is_symlink(),'missing/linked publication file'
def exact(path,entry):
 regular(path)
 assert digest(path)==entry['sha256'],'runtime archive checksum mismatch'
 if 'size_bytes' in entry:assert path.stat().st_size==entry['size_bytes'],'runtime archive size mismatch'
def install(source,target):
 assert not target.is_symlink(),'linked publication destination'
 if target.exists() and target.read_bytes()==source.read_bytes():return False
 fd,part=tempfile.mkstemp(prefix='.'+target.name+'.',dir=root)
 try:
  with os.fdopen(fd,'wb') as out,source.open('rb') as incoming:shutil.copyfileobj(incoming,out)
  os.chmod(part,0o644);os.replace(part,target)
 finally:
  Path(part).unlink(missing_ok=True)
 return True
try:
 with tempfile.TemporaryDirectory(prefix='weavelo-desktop-') as temp:
  temp=Path(temp)
  with tarfile.open(archive) as tar:
   names=set()
   for item in tar.getmembers():
    assert item.isfile() and Path(item.name).name==item.name and item.name not in names,'unexpected archive path/type'
    names.add(item.name)
    with tar.extractfile(item) as incoming,(temp/item.name).open('xb') as out:shutil.copyfileobj(incoming,out)
  doc=json.loads((temp/'desktop-releases.json').read_text())
  entries=doc['releases'];selected=[e for e in entries if scope=='all' or 'macos' in e['file'].lower()]
  assert len({e['file'] for e in entries})==len(entries),'duplicate runtime filenames'
  for entry in entries:assert Path(entry['file']).name==entry['file'] and entry['file'].endswith(('.zip','.tar.gz','.tgz'))
  acceptance=doc.get('acceptance_file','acceptance-2.0.0-alpha.2.html')
  assert Path(acceptance).name==acceptance
  metadata={} if scope=='signer' else {name:name for name in (acceptance,'index.html','desktop-releases.json')}
  signer_source='desktop-releases.json' if scope=='signer' else 'macos-signer-current.json'
  if signer_source in names:
   signer_doc=json.loads((temp/signer_source).read_text())
   assert signer_doc.get('schema')==1 and len(signer_doc['releases'])==1,'invalid signer manifest'
   signer_entry=signer_doc['releases'][0];signer=signer_entry.get('signer_sha1','');build=signer_entry.get('build','')
   assert re.fullmatch('[0-9a-f]{40}',signer),'invalid signer'
   assert re.fullmatch(r'\d+(?:\.\d+){0,2}',build),'invalid build'
   assert signer_entry.get('build_identity_expectations'),'missing verified source identity'
   mac=[entry for entry in selected if 'macos' in entry['file'].lower()]
   assert len(mac)==1 and dict(mac[0],signer_sha1=signer,build=build)==signer_entry,'signer manifest does not match the runtime release'
   if scope=='signer':assert len(entries)==1,'unexpected signer publication entries'
   destination='desktop-releases-macos-'+signer+'.json'
   metadata[destination]=signer_source
  else:assert scope!='signer','missing signer manifest'
  allowed={e['file'] for e in selected}|set(metadata.values())
  if scope=='macos':allowed|={'desktop-releases.before.json','index.before.html'}
  assert names==allowed,'unexpected or missing publication files'
  lock=root/'.desktop-publish.lock'
  assert not lock.is_symlink(),'linked publication lock'
  with lock.open('a') as held:
   fcntl.flock(held,fcntl.LOCK_EX)
   if scope=='macos':
    before_raw=(temp/'desktop-releases.before.json').read_bytes();before=json.loads(before_raw)
    assert len(selected)==1 and len([e for e in before['releases'] if 'macos' in e['file'].lower()])==1
    assert selected[0].get('build_identity_expectations'),'missing verified macOS identity pins'
    preserved=[e for e in before['releases'] if 'macos' not in e['file'].lower()]
    assert [e for e in entries if 'macos' not in e['file'].lower()]==preserved,'other platform entries changed'
    assert {k:v for k,v in doc.items() if k not in ('releases','published_at','acceptance_file')}=={k:v for k,v in before.items() if k not in ('releases','published_at','acceptance_file')},'unrelated publication metadata changed'
    assert acceptance!=before['acceptance_file'],'preserve previous acceptance page'
    regular(root/'desktop-releases.json');regular(root/'index.html')
    assert (root/'desktop-releases.json').read_bytes()==before_raw,'publication changed since read; stop and inspect'
    index_before=(temp/'index.before.html').read_bytes()
    assert (root/'index.html').read_bytes()==index_before,'index changed since read; stop and inspect'
    assert (temp/'index.html').read_bytes()==index_before.replace(before['acceptance_file'].encode(),acceptance.encode()),'unrelated index content changed'
    # Preserve and verify all previous runtime packages, including the rollback Mac.
    for entry in before['releases']:
     assert Path(entry['file']).name==entry['file']
     exact(root/entry['file'],entry)
    regular(root/before['acceptance_file'])
    if (root/acceptance).exists():
     regular(root/acceptance)
     assert (root/acceptance).read_bytes()==(temp/acceptance).read_bytes(),'immutable acceptance conflict'
   if signer_source in names:
    current=root/destination
    assert not current.is_symlink(),'linked signer manifest'
    if current.exists():
     regular(current);previous=json.loads(current.read_bytes())
     assert previous.get('schema')==1 and len(previous.get('releases',[]))==1,'invalid current signer manifest'
     old=previous['releases'][0]
     assert old.get('signer_sha1')==signer,'signer manifest identity conflict'
     assert re.fullmatch(r'\d+(?:\.\d+){0,2}',old.get('build','')),'invalid current build'
     def build_key(value):
      numbers=tuple(map(int,value.split('.')));return numbers+(0,)*(3-len(numbers))
     assert build_key(build)>=build_key(old['build']),'signer manifest downgrade refused'
     if build_key(build)==build_key(old['build']):assert signer_entry==old,'same build different release refused'
     assert Path(old['file']).name==old['file'],'invalid current artifact name'
     exact(root/old['file'],old)
   # Validate every incoming archive and immutable collision before any content write.
   for entry in selected:
    name=entry['file'];exact(temp/name,entry);target=root/name
    assert not target.is_symlink(),'linked runtime destination'
    if target.exists():assert digest(target)==entry['sha256'],'immutable artifact conflict'
   for name,source in metadata.items():
    regular(temp/source);assert not (root/name).is_symlink(),'linked metadata destination'
   for entry in selected:
    name=entry['file'];changed=install(temp/name,root/name)
    print('INSTALLED' if changed else 'UNCHANGED',name,entry['sha256'],flush=True)
   previous={name:(root/name).read_bytes() if (root/name).exists() else None for name in metadata}
   changed=[]
   try:
    for name,source in metadata.items():
     if install(temp/source,root/name):changed.append(name);print('INSTALLED',name,flush=True)
     else:print('UNCHANGED',name,flush=True)
   except BaseException:
    for name in reversed(changed):
     if previous[name] is None:(root/name).unlink()
     else:
      saved=temp/('restore-'+name);saved.write_bytes(previous[name]);install(saved,root/name)
    raise
finally:
 archive.unlink(missing_ok=True)
PY
fi
# Verify both the updated Mac and all preserved platform bytes over the original URL.
python3 - "$STAGE" <<'PY' > "$STAGE/desktop-sha256.txt"
import json,sys
from pathlib import Path
for entry in json.loads((Path(sys.argv[1])/'desktop-releases.json').read_text())['releases']:
 print(entry['sha256'],entry['file'])
PY
while read -r expected name; do
  curl -fLsS --connect-timeout 15 --max-time 300 "$BASE/$name" -o "$VERIFY"
  actual="$(shasum -a 256 "$VERIFY" | awk '{print $1}')"
  test "$actual" = "$expected" || { echo "Public checksum mismatch: $name" >&2; exit 1; }
  printf 'PUBLIC_VERIFIED %s/%s %s\n' "$BASE" "$name" "$actual"
done < "$STAGE/desktop-sha256.txt"
if [[ "$SCOPE" == signer ]]; then
  SIGNER="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["releases"][0]["signer_sha1"])' "$STAGE/desktop-releases.json")"
  MANIFEST="desktop-releases-macos-$SIGNER.json"
else
  MANIFEST=desktop-releases.json
fi
curl -fLsS --connect-timeout 15 --max-time 30 "$BASE/$MANIFEST" > "$STAGE/public-desktop-releases.json"
cmp "$STAGE/desktop-releases.json" "$STAGE/public-desktop-releases.json"
if [[ -f "$STAGE/macos-signer-current.json" ]]; then
  SIGNER="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["releases"][0]["signer_sha1"])' "$STAGE/macos-signer-current.json")"
  curl -fLsS --connect-timeout 15 --max-time 30 "$BASE/desktop-releases-macos-$SIGNER.json" > "$STAGE/public-signer-current.json"
  cmp "$STAGE/macos-signer-current.json" "$STAGE/public-signer-current.json"
fi
