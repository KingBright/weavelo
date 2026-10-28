#!/usr/bin/env python3
"""Version-bound independent Desktop upgrade; original first-install journal is immutable.

Plans are read-only. Unknown stop/exchange/start results are observed, never replayed.
Rollback requires the original transaction and all owners confirmed stopped.
No credentials, service configuration, retained clients or OS permissions are changed.
"""
import argparse
from contextlib import contextmanager
import ctypes
import fcntl
import json
import os
import re
from pathlib import Path
import select
import shutil
import stat
import subprocess
import sys
import time
import uuid
import zipfile

import build_identity_gate as gate
import linux_desktop_profile_launch as profile_launch
import install_linux_independent as fresh
from install_linux_desktop import sync
from upgrade_macos_desktop import private, read_record, atomic_record, atomic_bytes, fsync_tree, discard_matching_pending

def require(ok, text):
    if not ok: raise ValueError(text)

def safe(path, directory=False):
    path=Path(path)
    info=path.lstat()
    require(not stat.S_ISLNK(info.st_mode) and info.st_uid==os.getuid()
            and not info.st_mode & 0o022 and (stat.S_ISDIR(info.st_mode) if directory else
                stat.S_ISREG(info.st_mode) and info.st_nlink==1), 'Owned stable installation entry required')
    return info

def file_pin(path):
    before=safe(path)
    result={'sha256':gate.hash_file(path),'device':before.st_dev,'inode':before.st_ino,
            'mode':stat.S_IMODE(before.st_mode),'uid':before.st_uid,
            'xattrs':{n:gate.digest(os.getxattr(path,n)) for n in sorted(os.listxattr(path))}}
    after=safe(path)
    require((before.st_ino,before.st_size,before.st_mtime_ns,before.st_ctime_ns)==
            (after.st_ino,after.st_size,after.st_mtime_ns,after.st_ctime_ns),'Installation entry changed')
    return result

def image(path):
    safe(path,True)
    for p in path.rglob('*'):
        safe(p,p.is_dir())
    require({str(p.relative_to(path)) for p in path.rglob('*') if p.is_file()}==fresh.PAYLOAD,
            'Unknown application files retained without reading them')
    rows=gate.files_in(path)
    require({r['path'] for r in rows}==fresh.PAYLOAD,'Unexpected app payload retained; inspect before upgrade')
    dirs={str(p.relative_to(path)):{'device':p.stat().st_dev,'inode':p.stat().st_ino,'uid':p.stat().st_uid,'mode':stat.S_IMODE(p.stat().st_mode),'xattrs':{n:gate.digest(os.getxattr(p,n)) for n in sorted(os.listxattr(p))}} for p in [path,*path.rglob('*')] if p.is_dir()}
    return {'files':rows,'directories':dirs,'protection':{str(p.relative_to(path)):file_pin(p) for p in path.rglob('*') if p.is_file()}}

def security_labels(value):
    return {kind:{name:{k:v for k,v in row.get('xattrs',{}).items() if k.startswith(('security.','system.posix_acl'))}
                  for name,row in value[kind].items()} for kind in ('directories','protection')}

def profile_pin(profile):
    root=private(profile,True)
    require(not any((profile/n).exists() or (profile/n).is_symlink() for n in
        ('mesh.secret-recovery','.mesh.secret-recovery.pending','.mesh.secret-checkpoint.pending')),
        'Pending protected profile transaction; recover its original operation first')
    result={'directory':{'device':root.st_dev,'inode':root.st_ino,'uid':root.st_uid,'mode':stat.S_IMODE(root.st_mode)},'files':{}}
    for name in ('mesh.conf','mesh.secret','background-service.enabled'):
        p=profile/name;m=private(p)
        result['files'][name]={'device':m.st_dev,'inode':m.st_ino,'uid':m.st_uid,'mode':stat.S_IMODE(m.st_mode),
            'size':m.st_size,'mtime_ns':m.st_mtime_ns,'ctime_ns':m.st_ctime_ns,'links':m.st_nlink}
    # Deliberately no credential open/read/hash/copy or credential-derived verifier.
    return result

@contextmanager
def stopped_locks(profile):
    held=[]
    try:
        for path in (profile,profile/'desktop-instance.lock',profile/'gui-state/desktop-instance.lock'):
            m=private(path,path==profile)
            fd=os.open(path,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK);held.append(fd)
            require(os.fstat(fd).st_ino==m.st_ino,'Lifecycle lock changed')
            fcntl.flock(fd,fcntl.LOCK_EX|fcntl.LOCK_NB)
        yield
    finally:
        for fd in held: fcntl.flock(fd,fcntl.LOCK_UN);os.close(fd)

def check_mount_layout(paths):
    app=paths['app'];exe=app/'weavelo';work=paths['work']
    devices={p.stat().st_dev for p in (work,app.parent,app,exe)}
    require(len(devices)==1 and not os.path.ismount(app),'App/executable/staging mount boundary unsupported; no owner stopped')
    if sys.platform.startswith('linux'):
        # Also reject same-device bind mounts, which os.path.ismount cannot
        # distinguish. No unrelated mount data is returned or persisted.
        mounts=[]
        for line in Path('/proc/self/mountinfo').read_text().splitlines():
            fields=line.split(' - ',1)[0].split()
            require(len(fields)>=6 and fields[0].isdigit(),'Mount observation unavailable; no owner stopped')
            target=Path(re.sub(r'\\([0-7]{3})',lambda m:chr(int(m[1],8)),fields[4]))
            mounts.append((target,fields[0]))
            slots=(work/'upgrade/rollback-app',work/'current-update/candidate-app')
            require(not (target==app or app in target.parents or any(target==slot or slot in target.parents for slot in slots)),
                    'Mounted application/exchange subtree unsupported; no owner stopped')
        def mount_id(path):
            matches=[(len(target.parts),identifier) for target,identifier in mounts if target==path or target in path.parents]
            require(bool(matches),'Missing mount identity; no owner stopped')
            depth=max(item[0] for item in matches)
            ids={identifier for level,identifier in matches if level==depth}
            require(len(ids)==1,'Ambiguous mount stack retained; no owner stopped')
            return ids.pop()
        locations=[work,app.parent]
        if (work/'upgrade').exists():locations.append(work/'upgrade')
        if (work/'current-update').exists():locations.append(work/'current-update')
        require(len({mount_id(p) for p in locations})==1,
                'Exchange source/target belong to different mount IDs; no owner stopped')


def loaded_exec(document,executable,paths=None,*,profile_launch_preparing=False):
    require(set(document)=={'type','data'} and document['type']=='a(sasbttttuii)' and len(document['data'])==1,
            'One standard loaded Desktop ExecStart required')
    row=document['data'][0]
    if paths is not None and profile_launch.authorize(paths,preparing=profile_launch_preparing):
        require(len(row)==10 and row[2] is False and row[0]=='/usr/bin/env'
                and row[1]==profile_launch.arguments(paths),'Loaded canonical profile launcher differs')
        return {'executable':str(executable),'arguments':['--background-service'],
                'ignore_failure':False,'canonical_profile_launcher':profile_launch.arguments(paths)}
    require(len(row)==10 and row[2] is False and isinstance(row[0],str) and
            isinstance(row[1],list) and len(row[1])==2 and row[1][1]=='--background-service'
            and Path(row[0]).resolve(strict=True)==executable and Path(row[1][0]).resolve(strict=True)==executable,
            'Loaded Desktop executable/arguments differ; unknown values are not reported')
    return {'executable':str(executable),'arguments':['--background-service'],'ignore_failure':False}

EXEC_HOOKS=('ExecStartPre','ExecStartPost','ExecCondition','ExecStop','ExecStopPost')

def empty_exec(document):
    require(isinstance(document,dict) and set(document)=={'type','data'}
            and document['type']=='a(sasbttttuii)' and document['data']==[],
            'Loaded command hook is nonempty or unverified; no owner stopped')

FORWARD=('preparing','staged','gui-stop-intent','background-stop-intent','owners-stopped',
         'exchange-intent','exchanged','activation-intent','completed')
ROLLBACK=('rollback-exchange-intent','rollback-activation-intent','rolled-back')
RECORD_KIND='weavelo-desktop-linux-upgrade-transaction'

def exchange(first,second):
    require(first.parent.stat().st_dev==second.parent.stat().st_dev,'One-filesystem exchange required')
    libc=ctypes.CDLL(None,use_errno=True)
    require(hasattr(libc,'renameat2'),'Native exchange unavailable; no rename fallback')
    fn=libc.renameat2;fn.argtypes=[ctypes.c_int,ctypes.c_char_p,ctypes.c_int,ctypes.c_char_p,ctypes.c_uint]
    if fn(-100,os.fsencode(first),-100,os.fsencode(second),2):
        raise OSError(ctypes.get_errno(),'Desktop exchange did not confirm; inspect original tuple')
    sync(first.parent);sync(second.parent)

class Native:
    def __init__(self,paths):
        require(sys.platform.startswith('linux') and os.getuid()!=0,'Native ordinary-user Linux required')
        self.paths=paths;self.exe=paths['app']/'weavelo'
        self.env={k:v for k,v in os.environ.items() if not k.startswith(('WEAVELO_','REMOTE_PLAY_','RP_MEDIA_ACCEPTANCE'))}
        self.env.update(WEAVELO_DEVICE_GROUP_DIR=str(paths['profile']),WEAVELO_HOST_BIND_ADDR='0.0.0.0:39273',
                        WEAVELO_DISCOVERY_PORT='38119',WEAVELO_RELAY='0',WEAVELO_P2P='0')
    def ctl(self,*args):
        r=subprocess.run(['systemctl','--user',*args,'weavelo-desktop.service'],capture_output=True,text=True,timeout=15)
        require(r.returncode==0,'Original Desktop supervisor operation did not confirm')
        return r.stdout
    def exec_property(self,name):
        require(name in ('ExecStart',*EXEC_HOOKS),'Only fixed non-secret execution metadata may be queried')
        result=subprocess.run(['busctl','--user','--json=short','get-property','org.freedesktop.systemd1',
            '/org/freedesktop/systemd1/unit/weavelo_2ddesktop_2eservice','org.freedesktop.systemd1.Service',name],
            capture_output=True,text=True,timeout=15)
        require(result.returncode==0,'Loaded execution metadata unavailable')
        return json.loads(result.stdout)
    def stopped_state(self):
        r=subprocess.run(['systemctl','--user','show','--all','weavelo-desktop.service',
                          '-p','MainPID','-p','ActiveState','-p','Job'],capture_output=True,text=True,timeout=15)
        require(r.returncode==0,'Stopped supervisor observation unavailable')
        return dict(line.split('=',1) for line in r.stdout.splitlines())=={'MainPID':'0','ActiveState':'inactive','Job':''}
    def configuration(self,*,profile_launch_preparing=False):
        keys=['MainPID','ActiveState','SubState','UnitFileState','FragmentPath','DropInPaths','Restart','Job',
              'TriggeredBy','RestartForceExitStatus','ExecStop','ExecStopPost','OnSuccess','CPUWeight','TimeoutStopFailureMode',
              'NeedDaemonReload','ExecStartPre','ExecStartPost','ExecCondition']
        r=subprocess.run(['systemctl','--user','show','--all','weavelo-desktop.service',*[a for k in keys for a in ('-p',k)]],
                         capture_output=True,text=True,timeout=15)
        require(r.returncode==0,'Original service readback unavailable')
        fields=dict(line.split('=',1) for line in r.stdout.splitlines())
        missing=set(keys)-set(fields)
        require(set(fields)<=set(keys) and missing<=set(EXEC_HOOKS),'Missing service metadata is not presumed empty')
        for key in sorted(missing):
            empty_exec(self.exec_property(key));fields[key]=''
        require(set(fields)==set(keys) and fields['UnitFileState'] in ('enabled','disabled')
            and fields['Restart'] in ('no','on-failure')
            and all(not fields[k] for k in ('Job','TriggeredBy','RestartForceExitStatus','ExecStop','ExecStopPost','OnSuccess','ExecStartPre','ExecStartPost','ExecCondition'))
            and fields['NeedDaemonReload']=='no'
            and Path(fields['FragmentPath']).resolve()==self.paths['service'],'Unknown activation or stop policy')
        execution=loaded_exec(self.exec_property('ExecStart'),self.exe,self.paths,profile_launch_preparing=profile_launch_preparing)
        dropins={}
        for raw in fields['DropInPaths'].split():
            original=Path(raw);p=original.resolve(strict=True);m=p.lstat()
            require(original==p,'Linked service drop-in refused')
            require(m.st_uid in (0,os.getuid()) and stat.S_ISREG(m.st_mode) and not m.st_mode&0o022,'Untrusted service drop-in')
            if p.parent==self.paths['service'].parent/(self.paths['service'].name+'.d') and p.name in ('profile.conf','relay.conf'):
                dropins[str(p)]=dict(profile_launch.opaque_pin(p),opaque_environment_preserved=True)
                continue
            if p==profile_launch.location(self.paths):
                require(profile_launch.authorize(self.paths,preparing=profile_launch_preparing),'Unverified canonical launcher')
                dropins[str(p)]=profile_launch.opaque_pin(p)
                continue
            lines=[s.strip() for s in p.read_text().splitlines() if s.strip() and not s.lstrip().startswith(('#',';'))]
            require(lines and lines[0]=='[Service]' and all(s.startswith(('CPUWeight=','TimeoutStopFailureMode=')) for s in lines[1:]),
                    'Unreviewed service drop-in semantics')
            for line in lines[1:]:
                key,value=line.split('=',1)
                require((key=='TimeoutStopFailureMode' and value=='abort') or
                        (key=='CPUWeight' and (value=='idle' or value.isascii() and value.isdecimal() and 1<=int(value)<=10000)),
                        'Unreviewed service drop-in value')
            require(len(lines)==2,'Repeated service drop-in setting')
            dropins[str(p)]={'sha256':gate.hash_file(p),'device':m.st_dev,'inode':m.st_ino,'uid':m.st_uid,'mode':stat.S_IMODE(m.st_mode)}
        policy={k:v for k,v in fields.items() if k not in ('MainPID','ActiveState','SubState','DropInPaths','UnitFileState')}
        return {'enabled':fields['UnitFileState'],'dropins':dropins,'policy':dict(policy,execution=execution),'active':fields['ActiveState'],
                'substate':fields['SubState'],'pid':int(fields['MainPID']),
                'canonical_profile_launch':'canonical_profile_launcher' in execution}
    def frontend_busy(self):
        path=self.paths['profile']/'gui-state/desktop-instance.lock'
        before=private(path);private(path.parent,True)
        fd=os.open(path,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK)
        try:
            require((os.fstat(fd).st_dev,os.fstat(fd).st_ino)==(before.st_dev,before.st_ino),'Frontend lock changed')
            try:fcntl.flock(fd,fcntl.LOCK_EX|fcntl.LOCK_NB)
            except BlockingIOError:return True
            fcntl.flock(fd,fcntl.LOCK_UN)
            return False
        finally:os.close(fd)
    def gui_owner(self,sha):
        status=self.query('--gui-status-json')
        diagnostics=self.query('--gui-network-diagnostics-json')
        require(status.get('ok') is True and status.get('renderer')==gate.RENDERER
                and status.get('executable_sha256')==sha and diagnostics.get('ok') is True
                and diagnostics.get('executable_sha256')==sha and status.get('generation')==diagnostics.get('generation'),
                'GUI endpoint identity/generation conflict; no owner stopped')
        pid=diagnostics.get('pid')
        require(type(pid) is int and pid>1 and status.get('pid',pid)==pid,'Explicit native GUI PID required')
        gate.verify_running_linux(pid,self.exe,sha)
        return dict(status,pid=pid)
    def owners(self,sha):
        # Scope is the canonical Desktop profile, not every same-UID process.
        # Never request executable metadata for an unrelated protected process.
        config=self.configuration();pid=config['pid']
        if pid==0:
            require(config['active']=='inactive','Supervisor owner is still transitioning')
            with stopped_locks(self.paths['profile']):pass
            return set()
        require(type(pid) is int and pid>1,'Exact registered service PID required')
        gate.verify_running_linux(pid,self.exe,sha)
        found={pid}
        if self.frontend_busy():found.add(self.gui_owner(sha)['pid'])
        return found
    def query(self,*args):
        r=subprocess.run([str(self.exe),*args],env=self.env,capture_output=True,text=True,timeout=20)
        require(r.returncode==0,'Product command rejected or outcome unknown; inspect original state, do not replay')
        return json.loads(r.stdout)
    def observe(self,sha):
        pids=self.owners(sha);config=self.configuration()
        if not pids:
            require(config['pid']==0 and config['active']=='inactive','Supervisor still active or transitioning')
            with stopped_locks(self.paths['profile']):pass
            return {'background':None,'gui':None,'blockers':[],'active':False}
        bg=self.query('--background-status-json')
        require(bg['role']=='background-service' and bg['executable_sha256']==sha
                and bg['profile']==str(self.paths['profile']) and config['pid']==bg['pid']
                and config['active']=='active' and config['substate']=='running','Unknown background process')
        known={bg['pid']};gui=None;blockers=[]
        if pids!=known:
            gui=self.gui_owner(sha)
            pid=gui['pid'];known.add(pid)
            require(pids==known,'Additional unidentified Desktop owners')
            require(gui['sessions']['pending']==0 and gui['sessions']['sessions']==[] and gui['lease'] is None,
                    'GUI has active/pending sessions, transfers or lease')
            gui={'pid':pid,'generation':gui['generation'],'supported':gui.get('upgrade_quit')=='gui-upgrade-quit.v1'}
            if not gui['supported']:blockers.append('Current GUI requires its normal Quit GUI / Quit Weavelo control')
        if 'shutdown-idle.v1' not in bg.get('service_contract',{}).get('capabilities',[]):
            blockers.append('Current background lacks idle-only stop; normal Quit Weavelo must finish before apply')
        return {'background':{'pid':bg['pid'],'generation':bg['network_diagnostics']['generation']},
                'gui':gui,'blockers':blockers,'active':True}
    def stop_gui(self,before,sha):
        if before['gui']:
            r=self.query('--gui-upgrade-quit-json',before['gui']['generation'],sha)
            require(r.get('process_exit_observed') is True and r.get('cleanup_verified') is True,'GUI stop unconfirmed')
    def stop_background(self,before,sha):
        if before['background']:
            current=self.query('--background-status-json')
            require(current['pid']==before['background']['pid'] and current['executable_sha256']==sha
                    and current['network_diagnostics']['generation']==before['background']['generation'],'Background owner changed')
            r=self.query('--background-upgrade-stop-json',before['background']['generation'],sha,str(before['background']['pid']))
            require(r.get('idle_only') is True and r.get('process_exit_observed') is True and r.get('cleanup_verified') is True
                    and r.get('pid')==before['background']['pid'] and r.get('generation')==before['background']['generation']
                    and r.get('executable_sha256')==sha,
                    'Background stop unconfirmed')
        require(not self.owners(sha),'Desktop owner remains; no replacement')
        end=time.monotonic()+10
        while True:
            config=self.configuration()
            if config['active']=='inactive' and config['pid']==0:break
            require(time.monotonic()<end,'Service stopped state unconfirmed');time.sleep(.1)
    def start(self,before,sha):
        if before['background']:
            self.ctl('start')
            end=time.monotonic()+40
            while True:
                try:
                    owners=self.owners(sha);bg=self.query('--background-status-json')
                    if (bg.get('role')=='background-service' and bg.get('executable_sha256')==sha
                            and bg.get('profile')==str(self.paths['profile']) and bg.get('pid') in owners):break
                except (ValueError,OSError,gate.IdentityRejected,subprocess.TimeoutExpired):pass
                require(time.monotonic()<end,'Background activation unconfirmed');time.sleep(.2)
        if before['gui']:
            subprocess.Popen([str(self.exe),'--gui'],env=self.env,stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,start_new_session=True)
            end=time.monotonic()+40
            while True:
                try:
                    self.gui_owner(sha);break
                except (ValueError,OSError,gate.IdentityRejected,subprocess.TimeoutExpired):pass
                require(time.monotonic()<end,'GUI activation unconfirmed');time.sleep(.2)
        observed=self.observe(sha)
        require(all((observed[k] is None)==(before[k] is None) for k in ('background','gui')) and observed['active']==before['active'],
                'Restored runtime shape unconfirmed; inspect original operation')
        return observed

class Upgrade:
    def __init__(self,home,control=None):
        self.home=Path(home).resolve();self.paths=fresh.paths(self.home)
        for path in self.paths.values():
            for parent in path.parents:
                if parent==self.home.parent:break
                if parent.exists():safe(parent,True)
        self.work=self.paths['work'];private(self.work,True)
        check_mount_layout(self.paths)
        self.original=self.work/'state.json';self.transaction=self.work/'upgrade'
        self.journal=self.transaction/'state.json';self.slot=self.transaction/'rollback-app'
        self.control=control or Native(self.paths)
    @contextmanager
    def lock(self):
        fd=os.open(self.work,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
        try:fcntl.flock(fd,fcntl.LOCK_EX|fcntl.LOCK_NB);yield
        finally:fcntl.flock(fd,fcntl.LOCK_UN);os.close(fd)
    def invariants(self):
        original,sha=read_record(self.original)
        require(original['phase']=='installed-not-started' and original['plan_sha256']==fresh.plan_sha(original['plan'])
                and original['plan']['kind']=='weavelo-independent-linux-first-install'
                and original['plan']['paths']=={k:str(v) for k,v in self.paths.items()},'Original Desktop installation journal differs')
        return {'original_journal_sha256':sha,'entries':{k:file_pin(self.paths[k]) for k in ('launcher','service','desktop','icon')},
                'profile':profile_pin(self.paths['profile'])}
    def predecessor(self):
        require(not self.transaction.exists() and not self.transaction.is_symlink(),'Existing transaction retained; read status, do not replay')
        original,sha=read_record(self.original)
        return {'binary_sha256':original['plan']['binary_sha256'],'journal_sha256':sha}
    def startup_compatibility(self,key,archive,fixed,configuration):
        require(key=='service' and configuration.get('canonical_profile_launch') is True
                and profile_launch.authorize(self.paths),
                'Candidate changes existing startup/UI entry; separate plan required')
        with zipfile.ZipFile(archive) as source:
            stock=source.read('Weavelo/deploy/linux/weavelo-desktop.service')
        require(gate.digest(stock)==fixed['entries'][key]['sha256'],
                'Installed unit is not the exact verified package stock template; preserved')
        return 'retained-stock-template-with-canonical-launcher'
    def plan(self,archive,pins):
        check_mount_layout(self.paths)
        predecessor=self.predecessor()
        manifest=fresh.verify(Path(archive),*pins)
        fixed=self.invariants();old=image(self.paths['app'])
        old_sha=next(r['sha256'] for r in old['files'] if r['path']=='weavelo')
        require(old_sha==predecessor['binary_sha256'],'Installed committed binary differs')
        require(manifest['binary_sha256']!=old_sha,'Candidate is already installed')
        configuration=self.control.configuration();compatibility={}
        for key,(raw,_) in fresh.payloads(self.home,archive).items():
            if gate.digest(raw)==fixed['entries'][key]['sha256']:
                compatibility[key]='matching-rendered-template'
                continue
            compatibility[key]=self.startup_compatibility(key,archive,fixed,configuration)
        before=self.control.observe(old_sha)
        opaque=any(row.get('opaque_environment_preserved') for row in configuration['dropins'].values())
        require(not opaque or configuration.get('canonical_profile_launch') is True,
                'Opaque environment preserved; explicit canonical profile launcher required before upgrade')
        return {'schema':1,'kind':'weavelo-independent-desktop-linux-upgrade','home':str(self.home),
                'archive':str(Path(archive).resolve()),'pins':list(pins),'invariants':fixed,'old_image':old,
                'predecessor':predecessor,
                'old_sha256':old_sha,'new_sha256':manifest['binary_sha256'],'new_files':manifest['files'],
                'configuration':{k:v for k,v in configuration.items() if k in ('enabled','dropins','policy')},'runtime':before,
                'startup_entry_compatibility':compatibility,
                'applicable':not before['blockers'],'persistent_permissions_changed':False,'membership_changed':False}
    def same(self,plan):
        check_mount_layout(self.paths)
        require(self.invariants()==plan['invariants'],'Original journal/startup/profile changed; preserved')
        configuration=self.control.configuration()
        require({k:v for k,v in configuration.items() if k in ('enabled','dropins','policy')}==plan['configuration'],'Startup policy drift; preserved')
    def save(self,record,expected):
        return atomic_record(self.journal,record,expected)
    def begin(self,plan):
        self.transaction.mkdir(mode=0o700);sync(self.work)
        return None
    def finish(self,record,current):
        record['phase']='completed';self.save(record,current)
        plan=record['plan']
        return {'phase':'completed','transaction_id':record['transaction_id'],'source_commit':plan['pins'][2],
                'installed_sha256':plan['new_sha256'],'rollback_retained':True,'profile_preserved':True,'startup_preserved':True,'running':record['running']}
    def apply(self,plan,digest):
        require(gate.digest(gate.canonical(plan))==digest,'Exact reviewed plan required')
        with self.lock():
            require(self.plan(plan['archive'],plan['pins'])==plan,'Plan changed; no action')
            require(plan['applicable'],'Installed owners lack normal idle-only exit; no stop or file changes')
            previous=self.begin(plan)
            record={'schema':1,'kind':RECORD_KIND,'transaction_id':str(uuid.uuid4()),'plan':plan,'plan_sha256':digest,'phase':'preparing'}
            current=self.save(record,previous)
            self.slot.mkdir(mode=0o700)
            with zipfile.ZipFile(plan['archive']) as archive:
                for row in plan['new_files']:
                    raw=archive.read('Weavelo/'+row['path'])
                    require(len(raw)==row['bytes'] and gate.digest(raw)==row['sha256'],'Candidate bytes changed')
                    fresh.write_new(self.slot/row['path'],raw,0o755 if row['path']=='weavelo' else 0o644)
            fsync_tree(self.slot)
            require(gate.files_in(self.slot)==plan['new_files'],'Staged package differs')
            record['candidate_image']=image(self.slot)
            require(security_labels(record['candidate_image'])==security_labels(plan['old_image']),
                    'Candidate security labels/ACL differ; nothing was stopped and no security repair is attempted')
            record['phase']='staged';current=self.save(record,current)
            self.same(plan);require(image(self.paths['app'])==plan['old_image'],'Installed image changed before stop')
            require(self.control.observe(plan['old_sha256'])==plan['runtime'],'Owner changed before stop')
            record['phase']='gui-stop-intent';current=self.save(record,current)
            self.control.stop_gui(plan['runtime'],plan['old_sha256'])
            record['phase']='background-stop-intent';current=self.save(record,current)
            self.control.stop_background(plan['runtime'],plan['old_sha256'])
            record['phase']='owners-stopped';current=self.save(record,current)
            with stopped_locks(self.paths['profile']):
                self.same(plan);require(image(self.paths['app'])==plan['old_image'],'Installed image changed after exit')
                require(image(self.slot)==record['candidate_image'],'Staged app changed after exit')
                fsync_tree(self.paths['app'])
                record['phase']='exchange-intent';current=self.save(record,current)
                exchange(self.slot,self.paths['app'])
                require(image(self.slot)==plan['old_image'] and image(self.paths['app'])==record['candidate_image'],'Exchange tuple unconfirmed')
                self.same(plan);record['phase']='exchanged';current=self.save(record,current)
            record['phase']='activation-intent';current=self.save(record,current)
            record['running']=self.control.start(plan['runtime'],plan['new_sha256'])
            self.same(plan);require(image(self.paths['app'])==record['candidate_image'] and image(self.slot)==plan['old_image'],'Final app/rollback changed')
            return self.finish(record,current)
    def historical_runtime(self,runtime,plan):
        require(isinstance(runtime,dict) and set(runtime)=={'background','gui','blockers','active'}
                and type(runtime['active']) is bool and runtime['active']==plan['runtime']['active']
                and isinstance(runtime['blockers'],list),'Unknown historical activation receipt retained')
        allowed={'Current GUI requires its normal Quit GUI / Quit Weavelo control',
                 'Current background lacks idle-only stop; normal Quit Weavelo must finish before apply'}
        require(all(isinstance(item,str) and item in allowed for item in runtime['blockers']),'Unknown historical activation reason')
        for role in ('background','gui'):
            owner=runtime[role];previous=plan['runtime'][role]
            require((owner is None)==(previous is None),'Historical activation shape differs')
            if owner is None:continue
            expected={'pid','generation'}|({'supported'} if role=='gui' else set())
            require(set(owner)==expected and type(owner['pid']) is int and 1<owner['pid']<2**32
                    and isinstance(owner['generation'],str) and len(owner['generation'])<=128
                    and owner['generation'].split('-')[0]==str(owner['pid'])
                    and len(owner['generation'].split('-'))==2 and owner['generation'].split('-')[1].isdigit(),
                    'Unknown historical owner binding retained')
            if role=='gui':require(type(owner['supported']) is bool,'Unknown historical GUI capability')
        return runtime
    def validate_record(self,record):
        base={'schema','kind','transaction_id','plan','plan_sha256','phase'}
        require(set(record)<=base|{'candidate_image','running'} and base<=set(record)
                and type(record['schema']) is int and record['schema']==1 and record['kind']==RECORD_KIND
                and str(uuid.UUID(record['transaction_id']))==record['transaction_id']
                and record['phase'] in FORWARD+ROLLBACK,'Unknown transaction schema/fields/phase retained')
        plan=record['plan']
        require(plan['schema']==1 and plan['kind']=='weavelo-independent-desktop-linux-upgrade'
                and plan['home']==str(self.home) and gate.digest(gate.canonical(plan))==record['plan_sha256'],
                'Unknown transaction plan retained')
        compatibility=plan.get('startup_entry_compatibility',{})
        require(set(compatibility)=={'launcher','service','desktop','icon'}
                and all(value=='matching-rendered-template' or value=='retained-installed-entry' and plan.get('no_new_rollback') is True or key=='service' and
                    value=='retained-stock-template-with-canonical-launcher' for key,value in compatibility.items()),
                'Unknown startup compatibility proof retained')
        if compatibility['service']=='retained-stock-template-with-canonical-launcher':
            require(plan['configuration'].get('policy',{}).get('execution',{}).get('canonical_profile_launcher')
                    ==profile_launch.arguments(self.paths),'Unbound retained stock unit proof')
        candidate=record.get('candidate_image')
        if candidate is not None:
            require(set(candidate)=={'files','directories','protection'} and candidate['files']==plan['new_files']
                    and set(candidate['protection'])==fresh.PAYLOAD,'Unbound candidate image retained')
        require(record['phase'] not in FORWARD[1:] or candidate is not None,'Missing staged candidate proof')
        require('running' not in record or record['phase'] in ('completed',*ROLLBACK),'Unexpected activation proof retained')
        if 'running' in record:self.historical_runtime(record['running'],plan)
        return record
    def observed_record(self):
        record,sha=read_record(self.journal);self.validate_record(record)
        pending=self.journal.with_name('state.json.upgrade-tmp');raw=None
        if pending.exists() or pending.is_symlink():
            next_record,_=read_record(pending);self.validate_record(next_record)
            raw=(json.dumps(next_record,indent=2)+'\n').encode()
            require(gate.hash_file(pending)==gate.digest(raw),'Nonmatching pending representation retained')
            phase,following=record['phase'],next_record['phase']
            allowed=set()
            if phase in FORWARD and phase!='completed':allowed.add(FORWARD[FORWARD.index(phase)+1])
            if phase!='rolled-back':allowed.update(ROLLBACK[:2])
            if phase=='rollback-activation-intent':allowed.add('rolled-back')
            require(following in allowed,'Unknown pending phase transition retained')
            expected=dict(record,phase=following)
            if phase=='preparing' and following=='staged':
                candidate=next_record.get('candidate_image')
                require(candidate is not None and image(self.slot)==candidate,'Pending staged image lacks actual tuple proof')
                expected['candidate_image']=candidate
            if following in ('completed','rolled-back'):
                historical=self.historical_runtime(next_record.get('running'),record['plan'])
                actual_image=image(self.paths['app'])
                require(actual_image==(record.get('candidate_image') if following=='completed' else record['plan']['old_image']),
                        'Historical activation image tuple differs')
                expected['running']=historical
            require(next_record==expected,'Unknown pending record differences retained')
            if 'candidate_image' not in record and 'candidate_image' in next_record:record['candidate_image']=next_record['candidate_image']
        return record,sha,raw
    def status(self):
        if not self.journal.exists():return {'phase':'not-started','unrecorded_transaction':self.transaction.exists()}
        record,sha,_=self.observed_record();plan=record['plan']
        require(gate.digest(gate.canonical(plan))==record['plan_sha256'],'Transaction plan differs')
        live=image(self.paths['app']);slot=None;slot_state='absent'
        if self.slot.exists() or self.slot.is_symlink():
            try:slot=image(self.slot);slot_state='verified'
            except (ValueError,OSError,gate.IdentityRejected):slot_state='partial-or-unknown-retained'
        side='old' if live==plan['old_image'] else 'new' if live==record.get('candidate_image') else 'unknown'
        return {'phase':record['phase'],'transaction_id':record['transaction_id'],'journal_sha256':sha,'live_side':side,
                'slot_state':slot_state,'rollback_tuple_verified':slot is not None and slot==(plan['old_image'] if side=='new' else record.get('candidate_image')),
                'pending_metadata':self.journal.with_name('state.json.upgrade-tmp').exists(),
                'owners':sorted(self.control.owners(plan['old_sha256'] if side=='old' else plan['new_sha256'])) if side!='unknown' else None,
                'supervisor':self.control.configuration(),'forward_replay_supported':False}
    def recover(self,transaction_id):
        with self.lock():
            record,current,pending_raw=self.observed_record();plan=record['plan']
            require(record['transaction_id']==transaction_id and gate.digest(gate.canonical(plan))==record['plan_sha256'],'Original transaction required')
            self.same(plan)
            terminal=json.loads(pending_raw) if pending_raw is not None else record
            if terminal['phase']=='rolled-back':
                require(image(self.paths['app'])==plan['old_image'],'Terminal rollback app tuple differs; preserved')
                owners=sorted(self.control.owners(plan['old_sha256']))
                if pending_raw is not None:atomic_bytes(self.journal,pending_raw,current,recover_matching=True)
                return {'phase':'rolled-back','transaction_id':transaction_id,'terminal_checkpoint_confirmed':True,
                        'activation_replayed':False,'owners_now':owners,'profile_preserved':True}
            status=self.status();require(status['live_side'] in ('old','new'),'Unknown live image retained')
            sha=plan['old_sha256'] if status['live_side']=='old' else plan['new_sha256']
            require(not self.control.owners(sha),'Owners still live; observe or use normal product Quit before explicit recovery')
            configuration=self.control.configuration()
            require(configuration['active']=='inactive' and configuration['pid']==0,'Supervisor stop still unconfirmed; no recovery exchange')
            with stopped_locks(self.paths['profile']):
                self.same(plan)
                require(self.status()==status,'Transaction tuple changed before recovery')
                if pending_raw is not None:
                    discard_matching_pending(self.journal.with_name('state.json.upgrade-tmp'),pending_raw)
                if status['live_side']=='new':
                    require(status['rollback_tuple_verified'],'Unknown rollback image retained')
                    record['phase']='rollback-exchange-intent';current=self.save(record,current)
                    exchange(self.slot,self.paths['app'])
                require(image(self.paths['app'])==plan['old_image'],'Old image restoration unconfirmed')
                record['phase']='rollback-activation-intent';current=self.save(record,current)
            running=self.control.start(plan['runtime'],plan['old_sha256'])
            self.same(plan);require(image(self.paths['app'])==plan['old_image'],'Restored image changed')
            record['phase']='rolled-back';record['running']=running;self.save(record,current)
            return {'phase':'rolled-back','transaction_id':transaction_id,'profile_preserved':True,'running':running}

class CurrentUpgrade(Upgrade):
    """Repeat ordinary updates in one fixed staging area; retain historical evidence."""
    def __init__(self,home,control=None):
        super().__init__(home,control)
        self.historical=Upgrade(home,self.control)
        self.transaction=self.work/'current-update'
        self.journal=self.transaction/'state.json';self.slot=self.transaction/'candidate-app'

    def startup_compatibility(self,key,archive,fixed,configuration):
        # Binary-only updates do not install package startup templates. Current
        # launchers, UI entries and service policy remain pinned by same().
        return 'retained-installed-entry'

    def plan(self,archive,pins):
        return dict(super().plan(archive,pins),no_new_rollback=True)

    def historical_pin(self):
        old=self.historical
        if not old.transaction.exists() and not old.transaction.is_symlink():return None
        record,sha,pending=old.observed_record()
        require(pending is None and record['phase']=='completed','Historical update is unfinished; observe its original operation')
        require(image(old.slot)==record['plan']['old_image'],'Historical rollback tuple changed; retained')
        # Pairing/reload may legitimately replace protected profile inodes after
        # a completed update. Pin the current profile in the new plan instead.
        require(read_record(old.original)[1]==record['plan']['invariants']['original_journal_sha256'],
                'Original installation evidence changed; retained')
        return {'journal_sha256':sha,'rollback_image':image(old.slot)}

    def predecessor(self):
        historical=self.historical_pin()
        if self.transaction.exists() or self.transaction.is_symlink():
            private(self.transaction,True)
            record,sha,pending=self.observed_record()
            require(pending is None and record['phase']=='completed','Current update is unfinished; observe its original operation')
            require(image(self.paths['app'])==record['candidate_image'],'Current installed image changed')
            if self.slot.exists() or self.slot.is_symlink():
                require(image(self.slot)==record['plan']['old_image'],'Previous owned staging changed; retained')
            require({p.name for p in self.transaction.iterdir()}<= {'state.json','candidate-app'},'Unknown current staging entry retained')
            return {'binary_sha256':record['plan']['new_sha256'],'journal_sha256':sha,'historical':historical}
        if historical is not None:
            record,_,_=self.historical.observed_record()
            require(image(self.paths['app'])==record['candidate_image'],'Historical installed image changed')
            sha=record['plan']['new_sha256']
        else:
            original,_=read_record(self.original);sha=original['plan']['binary_sha256']
        return {'binary_sha256':sha,'journal_sha256':None,'historical':historical}

    def begin(self,plan):
        previous=plan['predecessor']['journal_sha256']
        if previous is None:
            require(not self.transaction.exists() and not self.transaction.is_symlink(),'Unrecorded staging retained')
            self.transaction.mkdir(mode=0o700);sync(self.work)
        else:
            record,sha,pending=self.observed_record()
            require(sha==previous and pending is None and record['phase']=='completed','Current journal changed')
            if self.slot.exists() or self.slot.is_symlink():self.discard_old_stage(record)
        return previous

    def same(self,plan):
        super().same(plan)
        require(self.historical_pin()==plan['predecessor']['historical'],'Historical evidence changed; retained')

    def discard_old_stage(self,record):
        require(image(self.paths['app'])==record['candidate_image'] and image(self.slot)==record['plan']['old_image'],
                'Owned staging tuple changed; retained')
        require(self.historical_pin()==record['plan']['predecessor']['historical'],'Historical evidence changed; retained')
        shutil.rmtree(self.slot);sync(self.transaction)

    def finish(self,record,current):
        result=super().finish(record,current)
        self.discard_old_stage(record)
        result.update(rollback_retained=False,historical_rollback_preserved=True)
        return result

    def recover(self,transaction_id):
        raise ValueError('Current updates retain their observed state for forward repair; no automatic rollback')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('archive',nargs='?',type=Path)
    for name in ('archive-sha256','manifest-sha256','commit','identity-sha256'):parser.add_argument('--expected-'+name)
    parser.add_argument('--expected-plan-sha256');parser.add_argument('--apply',action='store_true')
    parser.add_argument('--status',action='store_true');parser.add_argument('--recover-transaction')
    args=parser.parse_args();upgrade=CurrentUpgrade(Path.home())
    if (args.status or args.recover_transaction) and not upgrade.journal.exists():upgrade=upgrade.historical
    if args.status or args.recover_transaction:
        require(not args.archive and not args.apply,'Status/recovery cannot start an upgrade')
        result=upgrade.status() if args.status else upgrade.recover(args.recover_transaction)
    else:
        pins=[args.expected_archive_sha256,args.expected_manifest_sha256,args.expected_commit,args.expected_identity_sha256]
        require(args.archive and all(pins),'Archive and four independent pins required')
        plan=upgrade.plan(args.archive,pins);digest=gate.digest(gate.canonical(plan))
        result=upgrade.apply(plan,args.expected_plan_sha256) if args.apply else {'plan':plan,'plan_sha256':digest,'applied':False}
    print(json.dumps(result,indent=2))
if __name__=='__main__':main()
