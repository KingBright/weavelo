#!/usr/bin/env python3
"""Explicit Studio-local Desktop identity creation; no trust or installed-app changes.

Private inputs stay in a protected local stage and Security.framework. Passwords
never enter argv/stdout. An incomplete operation is preserved, never regenerated.
This is local persistent signing, not Apple Developer ID or notarization.
"""
import argparse
import ctypes as C
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import socket
import stat
import subprocess
import sys

LABEL = 'Weavelo Desktop Studio Code Signing'
APP_ID = 'fun.hackerlife.weavelo.desktop'
ROOT = Path.home()/'Library/Application Support/Weavelo/Signing'
KEYCHAIN = Path.home()/'Library/Keychains/login.keychain-db'


class SetupRejected(RuntimeError):
    pass


def require(ok, message):
    if not ok:
        raise SetupRejected(message)


def protected(path, directory=False):
    for p in (path, *path.parents):
        require(not p.is_symlink(), 'Linked signing path rejected')
    s = path.stat()
    require(s.st_uid == os.getuid() and stat.S_IMODE(s.st_mode) == (0o700 if directory else 0o600)
            and (stat.S_ISDIR(s.st_mode) if directory else stat.S_ISREG(s.st_mode) and s.st_nlink == 1),
            'Signing path owner/type/mode mismatch')


def write_new(path, value):
    fd = os.open(path, os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'w') as out:
        json.dump(value, out, indent=2); out.write('\n'); out.flush(); os.fsync(out.fileno())


def identities():
    p = subprocess.run(['/usr/bin/security','find-identity','-p','codesigning',str(KEYCHAIN)],
                       capture_output=True, text=True, timeout=20)
    require(p.returncode == 0, 'Cannot observe login keychain identities')
    return set(re.findall(r'^\s*\d+\)\s+([A-Fa-f0-9]{40})\s+"',p.stdout,re.M))


def status(root=ROOT):
    record = root/'identity.json'
    if not record.exists():
        return {'state':'not_recorded','scope':APP_ID,'root_present':root.exists()}
    protected(root,True); protected(record)
    data = json.loads(record.read_text())
    require(data.get('scope') == APP_ID and data.get('label') == LABEL
            and re.fullmatch('[0-9a-f]{40}',data.get('certificate_sha1','')), 'Wrong local identity record')
    cert = root/'certificate.pem'; protected(cert)
    require(hashlib.sha256(cert.read_bytes()).hexdigest() == data['certificate_file_sha256'],
            'Public certificate changed')
    require(data['certificate_sha1'].upper() in identities(), 'Recorded local identity unavailable; never regenerate')
    return dict(data,state='identity_present',trust_settings_changed=False,developer_id=False,notarized=False)


class Parameters(C.Structure):
    _fields_ = [('version',C.c_uint32),('flags',C.c_uint32),('passphrase',C.c_void_p),
                ('alertTitle',C.c_void_p),('alertPrompt',C.c_void_p),('accessRef',C.c_void_p),
                ('keyUsage',C.c_void_p),('keyAttributes',C.c_void_p)]


def import_identity(bundle, password):
    """Import only newly generated local bytes, with access limited to codesign."""
    s = C.CDLL('/System/Library/Frameworks/Security.framework/Security')
    cf = C.CDLL('/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation')
    P=C.c_void_p; refs=[]
    def api(lib,name,result,args):
        fn=getattr(lib,name);fn.restype=result;fn.argtypes=args;return fn
    release=api(cf,'CFRelease',None,[P])
    def keep(value):
        require(bool(value),'Native signing object unavailable');refs.append(value);return value
    def check(code,phase):
        require(code == 0,phase+' rejected (OSStatus '+str(code)+'); no permission fallback')
    string=api(cf,'CFStringCreateWithCString',P,[P,C.c_char_p,C.c_uint32])
    data=api(cf,'CFDataCreate',P,[P,P,C.c_long])
    array=api(cf,'CFArrayCreate',P,[P,P,C.c_long,P])
    try:
        # A missing keychain authorization fails visibly; do not drive/approve UI.
        check(api(s,'SecKeychainSetUserInteractionAllowed',C.c_int32,[C.c_bool])(False),'interaction mode')
        kc=P();check(api(s,'SecKeychainOpen',C.c_int32,[C.c_char_p,C.POINTER(P)])(str(KEYCHAIN).encode(),C.byref(kc)),'open login keychain');keep(kc.value)
        app=P();check(api(s,'SecTrustedApplicationCreateFromPath',C.c_int32,[C.c_char_p,C.POINTER(P)])(b'/usr/bin/codesign',C.byref(app)),'codesign application');keep(app.value)
        apps=keep(array(None,(P*1)(app.value),1,C.addressof(C.c_byte.in_dll(cf,'kCFTypeArrayCallBacks'))))
        label=keep(string(None,LABEL.encode(),0x08000100));access=P()
        check(api(s,'SecAccessCreate',C.c_int32,[P,P,C.POINTER(P)])(label,apps,C.byref(access)),'new-key codesign access');keep(access.value)
        phrase=keep(string(None,password.encode(),0x08000100))
        raw=bundle.read_bytes()
        payload=keep(data(None,C.cast(C.c_char_p(raw),P),len(raw)))
        fmt,kind,result=C.c_uint32(12),C.c_uint32(5),P()
        params=Parameters(passphrase=phrase,accessRef=access.value)
        check(api(s,'SecItemImport',C.c_int32,[P,P,C.POINTER(C.c_uint32),C.POINTER(C.c_uint32),C.c_uint32,C.POINTER(Parameters),P,C.POINTER(P)])(
            payload,None,C.byref(fmt),C.byref(kind),0,C.byref(params),kc,C.byref(result)),'import new Desktop identity')
        if result.value:keep(result.value)
    finally:
        for value in reversed(refs):release(value)


def finish(root, der, envelopes):
    sha1=hashlib.sha1(der).hexdigest();cert=root/'certificate.pem';stage=root/'.creation'
    require(sha1.upper() in identities(),'Imported identity not observed; stage retained, do not retry import')
    expected={stage/'private.pem',*envelopes}
    require(set(stage.iterdir())==expected,'Unexpected creation-stage files; preserve for inspection')
    for path in expected:protected(path)
    record={'schema':1,'scope':APP_ID,'label':LABEL,'certificate_sha1':sha1,
        'certificate_sha256':hashlib.sha256(der).hexdigest(),'certificate_file_sha256':hashlib.sha256(cert.read_bytes()).hexdigest(),
        'keychain':str(KEYCHAIN),'private_key_export_retained':False,'codesign_access':'/usr/bin/codesign',
        'old_identity_replaced':False,'trust_settings_changed':False,'developer_id':False,'notarized':False}
    for path in expected:path.unlink()
    stage.rmdir();write_new(root/'identity.json',record)
    return status(root)


def resume_format(root=ROOT):
    """One format repair for the same locally generated certificate/key, no new identity."""
    require(sys.platform=='darwin' and socket.gethostname().lower()=='macstudio.local' and root==ROOT,
            'Explicit Studio format repair only')
    protected(root,True);stage=root/'.creation';protected(stage,True)
    key=stage/'private.pem';cert=root/'certificate.pem'
    for p in (key,cert,root/'certificate-record.json'):protected(p)
    require(not (root/'identity.json').exists(),'Identity already recorded; use status')
    record=json.loads((root/'certificate-record.json').read_text())
    openssl=shutil.which('openssl');require(openssl is not None,'OpenSSL unavailable')
    der=subprocess.check_output([openssl,'x509','-in',str(cert),'-outform','DER'],timeout=15)
    require(record.get('scope')==APP_ID and record.get('certificate_sha1')==hashlib.sha1(der).hexdigest()
            and record.get('certificate_sha256')==hashlib.sha256(der).hexdigest(),'Original public certificate pin changed')
    public_cert=subprocess.check_output([openssl,'x509','-in',str(cert),'-pubkey','-noout'],timeout=15)
    public_key=subprocess.check_output([openssl,'pkey','-in',str(key),'-pubout'],timeout=15)
    require(public_cert==public_key,'Prepared private key does not match the pinned public certificate')
    require(record['certificate_sha1'].upper() not in identities(),'Identity may already be imported; inspect before repair')
    intent=root/'format-repair-intent.json'
    require(not intent.exists(),'Format repair already attempted; observe original state, do not repeat')
    write_new(intent,{'scope':APP_ID,'certificate_sha1':record['certificate_sha1'],
        'previous_native_status':-25264,'key_regenerated':False,'state':'single_format_repair_started'})
    bundle=stage/'supported-import.p12';password=secrets.token_hex(32)
    mask=os.umask(0o077)
    try:
        result=subprocess.run([openssl,'pkcs12','-export','-inkey',str(key),'-in',str(cert),'-out',str(bundle),
            '-keypbe','PBE-SHA1-3DES','-certpbe','PBE-SHA1-3DES','-macalg','sha1','-passout','stdin'],
            input=(password+'\n').encode(),stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=30)
        require(result.returncode==0,'Compatible envelope preparation failed; original stage retained')
        protected(bundle);import_identity(bundle,password)
        return finish(root,der,[stage/'identity.p12',bundle])
    finally:os.umask(mask)


def create(root=ROOT):
    require(sys.platform == 'darwin' and socket.gethostname().lower() == 'macstudio.local', 'Explicit Studio creation only')
    require(root == ROOT, 'Fixed Desktop signing directory required')
    if (root/'identity.json').exists():return status(root)
    require(not root.exists(), 'Incomplete/unrecorded identity stage exists; inspect it, never regenerate')
    # Refuse an existing same-project identity before generating another key.
    p=subprocess.run(['/usr/bin/security','find-certificate','-a','-c',LABEL,'-Z',str(KEYCHAIN)],capture_output=True,text=True,timeout=20)
    require(p.returncode in (0,44) and 'SHA-1 hash:' not in p.stdout,'Existing Desktop certificate or unreadable inventory; inspect before creation')
    for parent in root.parents:require(not parent.is_symlink(),'Linked signing parent')
    root.mkdir(mode=0o700,parents=True);protected(root,True)
    write_new(root/'creation-intent.json',{'schema':1,'scope':APP_ID,'label':LABEL,'pid':os.getpid(),
        'state':'creation_started','trust_changes_authorized':False,'old_identity_migration':False})
    stage=root/'.creation';stage.mkdir(mode=0o700)
    openssl=shutil.which('openssl');require(openssl is not None,'OpenSSL unavailable')
    key=stage/'private.pem';cert=root/'certificate.pem';bundle=stage/'identity.p12'
    old_mask=os.umask(0o077)
    try:
        args=[openssl,'req','-x509','-newkey','rsa:3072','-nodes','-days','3650','-sha256',
              '-subj','/CN='+LABEL+'/O=Weavelo Desktop',
              '-addext','basicConstraints=critical,CA:FALSE','-addext','keyUsage=critical,digitalSignature',
              '-addext','extendedKeyUsage=codeSigning','-keyout',str(key),'-out',str(cert)]
        require(subprocess.run(args,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=45).returncode==0,'Local certificate generation failed; stage retained')
        protected(key);protected(cert)
        password=secrets.token_hex(32)
        # Pipe-only password: never argv, environment, a password file, or stdout.
        result=subprocess.run([openssl,'pkcs12','-export','-inkey',str(key),'-in',str(cert),
            '-out',str(bundle),'-passout','stdin'],input=(password+'\n').encode(),
            stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=30)
        require(result.returncode==0,'Local import envelope preparation failed; stage retained')
        protected(bundle)
        der=subprocess.check_output([openssl,'x509','-in',str(cert),'-outform','DER'],timeout=15)
        sha1=hashlib.sha1(der).hexdigest()
        write_new(root/'certificate-record.json',{'scope':APP_ID,'certificate_sha1':sha1,'certificate_sha256':hashlib.sha256(der).hexdigest()})
        import_identity(bundle,password)
        return finish(root,der,[bundle])
    finally:
        os.umask(old_mask)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action',choices=('status','create','resume-format'));args=parser.parse_args()
    try:print(json.dumps({'create':create,'status':status,'resume-format':resume_format}[args.action](),indent=2))
    except (SetupRejected,OSError,subprocess.SubprocessError) as error:
        # Never print subprocess output, native payloads or passphrases.
        print(json.dumps({'state':'rejected','reason':str(error) if isinstance(error,SetupRejected) else type(error).__name__,
            'stage_preserved':ROOT.exists(),'retry':'inspect_original_state_before_retry'}));raise SystemExit(1)


if __name__=='__main__':main()
