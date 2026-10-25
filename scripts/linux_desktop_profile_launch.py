#!/usr/bin/env python3
"""Pin only the public Desktop profile at exec; never inspect ambient credentials.

This is a separate, reversible startup correction. Application upgrades retain it.
Unknown creation/reload results are observed or rolled back, never replayed.
"""
import argparse
import json
import os
from pathlib import Path
import stat
import subprocess
import sys

import build_identity_gate as gate
import install_linux_independent as fresh
from install_linux_desktop import sync
from upgrade_macos_desktop import private, read_record, atomic_record, atomic_bytes, discard_matching_pending

NAME='zz-weavelo-profile-exec.conf'
KIND='weavelo-linux-desktop-profile-launch'

def require(ok,message):
    if not ok:raise ValueError(message)

def location(paths):return paths['service'].parent/(paths['service'].name+'.d')/NAME
def journal(paths):return paths['work']/'profile-launch.json'

def quote(value):
    require(not any(ord(c)<32 for c in value),'Control character in public launch path')
    return '"'+value.replace('\\','\\\\').replace('"','\\"').replace('$','$$').replace('%','%%')+'"'

def arguments(paths):
    return ['/usr/bin/env','WEAVELO_DEVICE_GROUP_DIR='+str(paths['profile']),str(paths['app']/'weavelo'),'--background-service']

def content(paths):
    args=arguments(paths)
    return ('[Service]\nExecStart=\nExecStart='+args[0]+' '+quote(args[1])+' '+quote(args[2])+' '+args[3]+'\n').encode()

def opaque_pin(path):
    """Stat only: no value, digest, or copy of an environment file."""
    p=Path(path);m=private(p)
    require(not p.is_symlink(),'Linked startup configuration refused')
    directory(p.parent)
    return {'device':m.st_dev,'inode':m.st_ino,'uid':m.st_uid,'mode':stat.S_IMODE(m.st_mode),
            'size':m.st_size,'mtime_ns':m.st_mtime_ns,'ctime_ns':m.st_ctime_ns,'links':m.st_nlink}

def directory(path):
    m=path.lstat()
    require(stat.S_ISDIR(m.st_mode) and m.st_uid==os.getuid() and not m.st_mode&0o022,'Untrusted startup directory')

def authorize(paths,*,preparing=False):
    p=location(paths);record=journal(paths)
    if not p.exists() and not p.is_symlink():return False
    private(p);require(not p.is_symlink() and p.read_bytes()==content(paths),'Unknown profile launcher retained')
    r,_=read_record(record)
    pending=record.with_name(record.name+'.upgrade-tmp')
    require(not pending.exists() and not pending.is_symlink(),'Pending launch result is not usable authorization')
    require(r.get('kind')==KIND and r.get('schema')==1 and r.get('phase') in (('reload-intent','completed') if preparing else ('completed',))
        and r['plan']['home']==str(paths['profile'].parents[2])
        and r['plan']['content_sha256']==gate.digest(content(paths))
        and r['plan_sha256']==gate.digest(gate.canonical(r['plan'])),'Unbound profile launcher retained')
    require(r.get('file_pin')==opaque_pin(p),'Profile launcher identity changed')
    return True

class Correction:
    def __init__(self,home,control=None):
        import upgrade_linux_desktop as up
        self.up=up.Upgrade(home,control);self.paths=self.up.paths;self.file=location(self.paths);self.record=journal(self.paths)
        self.control=self.up.control
    def stable(self):
        c=self.control.configuration()
        return {'invariants':self.up.invariants(),'configuration':{k:c[k] for k in ('enabled','dropins','policy')}}
    def plan(self):
        require(not self.file.exists() and not self.file.is_symlink() and not self.record.exists()
                and not self.record.is_symlink(),'Existing profile launch operation; read its status')
        require(not self.up.transaction.exists(),'Application upgrade exists; observe its original operation')
        directory(self.file.parent)
        before=self.stable()
        return {'schema':1,'kind':KIND,'home':str(self.up.home),'before':before,'path':str(self.file),
                'content_sha256':gate.digest(content(self.paths)),'arguments':arguments(self.paths),
                'original_files_modified':False,'credential_values_read':False}
    def reload(self):
        r=subprocess.run(['systemctl','--user','daemon-reload'],capture_output=True,timeout=20)
        require(r.returncode==0,'Reload unconfirmed; inspect original operation, do not replay')
    def save(self,r,expected):return atomic_record(self.record,r,expected)
    def original_dropins(self,plan):
        for name,expected in plan['before']['configuration']['dropins'].items():
            p=Path(name)
            if expected.get('opaque_environment_preserved'):
                actual=dict(opaque_pin(p),opaque_environment_preserved=True)
            else:
                m=p.lstat()
                require(stat.S_ISREG(m.st_mode) and not p.is_symlink(),'Original public drop-in changed')
                actual={'sha256':gate.hash_file(p),'device':m.st_dev,'inode':m.st_ino,'uid':m.st_uid,'mode':stat.S_IMODE(m.st_mode)}
            require(actual==expected,'Original drop-in changed; preserved')
    def apply(self,plan,digest):
        require(gate.digest(gate.canonical(plan))==digest,'Exact public launch plan required')
        with self.up.lock():
            require(self.plan()==plan,'Startup state changed')
            r={'schema':1,'kind':KIND,'phase':'create-intent','plan':plan,'plan_sha256':digest};v=self.save(r,None)
            fresh.write_new(self.file,content(self.paths),0o600);sync(self.file.parent)
            r['phase']='reload-intent';r['file_pin']=opaque_pin(self.file);v=self.save(r,v)
            self.reload()
            require(self.up.invariants()==plan['before']['invariants'],'Original startup/profile changed')
            current=self.control.configuration(profile_launch_preparing=True);old=plan['before']['configuration']
            require(current['canonical_profile_launch'] is True and current['enabled']==old['enabled']
                    and {k:x for k,x in current['dropins'].items() if k!=str(self.file)}==old['dropins'],
                    'Loaded launch correction unconfirmed')
            expected=dict(old['policy'],execution=current['policy']['execution'])
            require(current['policy']==expected,'Other loaded service policy changed')
            r['phase']='completed';r['file_pin']=opaque_pin(self.file);self.save(r,v)
            return self.status()
    def status(self):
        r,sha=read_record(self.record)
        require(r['kind']==KIND and r['schema']==1 and gate.digest(gate.canonical(r['plan']))==r['plan_sha256']
                and r['plan']['home']==str(self.up.home),'Unknown launch operation retained')
        exists=self.file.exists() or self.file.is_symlink();matching=False
        if exists:
            private(self.file);require(not self.file.is_symlink(),'Linked launch file retained')
            matching=self.file.read_bytes()==content(self.paths)
        return {'kind':KIND,'phase':r['phase'],'record_sha256':sha,'matching_owned_file':matching,
                'file_exists':exists,'pending_metadata':self.record.with_name(self.record.name+'.upgrade-tmp').exists()
                    or self.record.with_name(self.record.name+'.upgrade-tmp').is_symlink(),
                'forward_replay_supported':False,'original_files_modified':False}
    def remove(self,digest):
        # Backward-only. Never activate a service after restoring its old entry.
        with self.up.lock():
            r,v=read_record(self.record);require(v==digest,'Exact launch status required')
            s=self.status()
            require(not self.up.transaction.exists(),'Retained application transaction protects its startup configuration')
            require(self.up.invariants()==r['plan']['before']['invariants'],'Original startup/profile changed')
            self.original_dropins(r['plan'])
            require(self.control.stopped_state(),'Owners not confirmed stopped')
            require(not s['file_exists'] or s['matching_owned_file'],'Unknown launch file retained')
            with __import__('upgrade_linux_desktop').stopped_locks(self.paths['profile']):
                # PID zero/inactive is separately verified; locks alone are insufficient.
                require(self.control.stopped_state(),'Owners not confirmed stopped')
                self.original_dropins(r['plan'])
                s=self.status()
                require(not s['file_exists'] or s['matching_owned_file'],'Unknown launch file retained')
                expected_pin=r.get('file_pin')
                if s['pending_metadata']:
                    pending=self.record.with_name(self.record.name+'.upgrade-tmp');next_record,_=read_record(pending)
                    raw=(json.dumps(next_record,indent=2)+'\n').encode()
                    require(gate.hash_file(pending)==gate.digest(raw),'Unknown pending representation retained')
                    if next_record.get('phase')=='removed':
                        require(r['phase']=='remove-intent' and next_record==dict(r,phase='removed')
                                and not s['file_exists'] and self.stable()==r['plan']['before'],
                                'Terminal removal tuple unconfirmed')
                        atomic_bytes(self.record,raw,v,recover_matching=True)
                        return self.status()
                    allowed={'create-intent':('reload-intent','remove-intent'),'reload-intent':('completed','remove-intent'),
                             'completed':('remove-intent',),'remove-intent':(),'removed':()}
                    expected=dict(r,phase=next_record.get('phase'))
                    if r['phase']=='create-intent' and next_record.get('phase')=='reload-intent':
                        require(s['matching_owned_file'],'Pending launcher absent')
                        expected['file_pin']=opaque_pin(self.file)
                        expected_pin=next_record.get('file_pin')
                    require(next_record.get('phase') in allowed.get(r['phase'],()) and next_record==expected,
                            'Unknown pending launch record retained')
                    discard_matching_pending(pending,raw)
                if s['file_exists']:
                    require(expected_pin is not None and opaque_pin(self.file)==expected_pin,
                            'Unrecorded or replaced launcher retained')
                if r['phase']=='removed':
                    require(not s['file_exists'] and self.stable()==r['plan']['before'],'Terminal removal changed')
                    return self.status()
                restored=False
                if not s['file_exists']:
                    try:restored=self.stable()==r['plan']['before']
                    except (ValueError,OSError):pass
                if s['file_exists']:r['file_pin']=expected_pin
                r['phase']='remove-intent';v=self.save(r,v)
                if s['file_exists']:
                    require(self.status()['matching_owned_file'] and opaque_pin(self.file)==expected_pin,
                            'Launcher changed before removal; retained')
                    os.unlink(self.file);sync(self.file.parent)
                if not restored:self.reload()
                require(self.stable()==r['plan']['before'],'Original loaded startup restoration unconfirmed')
                r['phase']='removed';self.save(r,v)
                return self.status()

def main():
    require(sys.platform.startswith('linux') and os.getuid()!=0,'Native ordinary-user Linux required')
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--apply',action='store_true');p.add_argument('--status',action='store_true')
    p.add_argument('--remove',action='store_true');p.add_argument('--expected-sha256');a=p.parse_args();c=Correction(Path.home())
    require(sum((a.apply,a.status,a.remove))<=1,'One launch operation required')
    if a.status:r=c.status()
    elif a.remove:r=c.remove(a.expected_sha256)
    else:
        plan=c.plan();digest=gate.digest(gate.canonical(plan));r=c.apply(plan,a.expected_sha256) if a.apply else {'plan':plan,'plan_sha256':digest,'applied':False}
    print(json.dumps(r,indent=2))
if __name__=='__main__':main()
