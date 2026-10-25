#!/usr/bin/env python3
"""One normal close/interrupt for the reviewed, installed pre-upgrade Desktop.

Not idle-only: old releases cannot freeze all new work. Refuses visible active
work and never escalates/repeats a request after an unknown result. No GUI input,
security setting, credential or persistent window-manager script is involved.
"""
import argparse
import json
import os
from pathlib import Path
import re
import select
import signal
import subprocess
import sys
import tempfile
import time
import uuid

import build_identity_gate as gate
import linux_desktop_profile_launch as launch
import upgrade_linux_desktop as up
from upgrade_macos_desktop import private, read_record, atomic_record

SOURCE='26a9c0c21d9e74fa293f07c54e5a70ebbabdce37'
SHA='aded05f172f8caf87f60ee9d8cbc9cc5ae3b73521f5cff5c8778fec5f32b5d81'
APPID='fun.hackerlife.weavelo.desktop'
KIND='weavelo-reviewed-legacy-normal-exit'

def require(ok,message):
    if not ok:raise ValueError(message)

def target(pid,exe,sha):
    require(type(pid) is int and pid>1 and pid!=os.getpid(),'Exact foreign owner PID required')
    gate.verify_running_linux(pid,exe,sha)
    p=Path('/proc')/str(pid);m=p.stat();row=(p/'stat').read_text().rsplit(')',1)[1].split()
    require(m.st_uid==os.getuid() and len(row)>19,'Foreign or incomplete process identity')
    return {'pid':pid,'uid':m.st_uid,'start_ticks':row[19],'executable':str(exe),'executable_sha256':sha}

def qdbus(*args):
    r=subprocess.run(['qdbus','org.kde.KWin',*args],capture_output=True,text=True,timeout=20)
    require(r.returncode==0,'KWin request unconfirmed; inspect original operation')
    return r.stdout.strip()

def script(source,label):
    require(qdbus('/Scripting','org.kde.kwin.Scripting.isScriptLoaded',label)=='false','Unknown existing window-manager task retained')
    fd,name=tempfile.mkstemp(prefix='weavelo-normal-exit-',suffix='.js');os.fchmod(fd,0o600)
    with os.fdopen(fd,'w') as f:f.write(source);f.flush();os.fsync(f.fileno())
    loaded=False
    try:
        sid=qdbus('/Scripting','org.kde.kwin.Scripting.loadScript',name,label)
        require(sid.isdigit(),'Window-manager script admission unconfirmed');loaded=True
        qdbus('/Scripting/Script'+sid,'org.kde.kwin.Script.run')
    finally:
        try:
            if loaded:qdbus('/Scripting','org.kde.kwin.Scripting.unloadScript',label)
        finally:os.unlink(name)

def window(pid):
    marker='WEAVELO_WINDOW_METADATA_'+uuid.uuid4().hex
    source=('var found=workspace.windowList().filter(function(w){return w.pid==='+str(pid)+';})'
        '.map(function(w){return {pid:w.pid,id:String(w.internalId),resourceClass:String(w.resourceClass),'
        'resourceName:String(w.resourceName),closeable:w.closeable,normalWindow:w.normalWindow,dialog:w.dialog};});'
        'console.warn('+json.dumps(marker+' ')+'+JSON.stringify(found));')
    script(source,'weavelo-metadata-'+uuid.uuid4().hex)
    r=subprocess.run(['journalctl','--user','--since','1 minute ago','--no-pager','-o','cat','--grep',marker],capture_output=True,text=True,timeout=20)
    require(r.returncode==0,'Target-only window metadata not observed')
    rows=[line.split(marker+' ',1)[1] for line in r.stdout.splitlines() if marker+' ' in line]
    require(len(rows)==1,'Ambiguous window metadata receipt')
    found=json.loads(rows[0]);require(len(found)==1,'One exact normal GUI window required')
    w=found[0]
    require(w['pid']==pid and w['resourceClass']==APPID and w['resourceName']=='weavelo'
        and w['closeable'] is True and w['normalWindow'] is True and w['dialog'] is False
        and re.fullmatch(r'\{[0-9a-f-]{36}\}',w['id']),'Unknown or non-closeable GUI window')
    return w

def close(w):
    # This is the normal compositor close request, never killWindow or input.
    source=('var found=workspace.windowList().filter(function(w){return w.pid==='+str(w['pid'])
        +' && String(w.internalId)==='+json.dumps(w['id'])+' && String(w.resourceClass)==='+json.dumps(APPID)
        +' && w.normalWindow && !w.dialog && w.closeable;});'
        'if(found.length!==1){throw new Error("Exact Weavelo window changed");}found[0].closeWindow();')
    script(source,'weavelo-close-'+uuid.uuid4().hex)

class Observer:
    def __init__(self,identity):
        self.identity=identity;self.fd=os.pidfd_open(identity['pid'],0)
        try:self.verify()
        except BaseException:os.close(self.fd);raise
    def verify(self):
        i=self.identity
        require(target(i['pid'],Path(i['executable']),i['executable_sha256'])==i,'Owner generation changed')
        require(not select.select([self.fd],[],[],0)[0],'Original owner already exited')
    def interrupt(self):
        self.verify();signal.pidfd_send_signal(self.fd,signal.SIGINT,None,0)
    def wait(self):require(bool(select.select([self.fd],[],[],30)[0]),'Normal exit unconfirmed; request must not be repeated')
    def release(self):os.close(self.fd)

class Quit:
    def __init__(self,home,control=None):
        self.up=up.Upgrade(home,control);self.n=self.up.control;self.paths=self.up.paths
        self.record=self.paths['work']/'legacy-normal-exit.json'
    def configuration(self):
        c=self.n.configuration()
        return {k:c[k] for k in ('enabled','dropins','policy')}
    def plan(self):
        require(not self.record.exists() and not self.record.is_symlink(),'Existing normal-exit request; read its status')
        require(not self.up.transaction.exists(),'Application upgrade already exists; inspect original operation')
        require(launch.authorize(self.paths),'Canonical restart route required before normal exit')
        exe=self.paths['app']/'weavelo';require(gate.hash_file(exe)==SHA,'Only the reviewed old executable is supported')
        info=self.n.query('--product-info-json')
        require(info.get('build_identity',{}).get('source',{}).get('commit')==SOURCE,'Only the reviewed old source is supported')
        state=self.n.observe(SHA);require(state['background'] is not None and state['gui'] is not None,'Both original owners must be present')
        return {'schema':1,'kind':KIND,'home':str(self.up.home),'invariants':self.up.invariants(),'configuration':self.configuration(),
                'runtime':state,'gui':target(state['gui']['pid'],exe,SHA),'background':target(state['background']['pid'],exe,SHA),
                'window':window(state['gui']['pid']),'legacy_source':SOURCE,'idle_only_guarantee':False}
    def apply(self,p,digest):
        require(gate.digest(gate.canonical(p))==digest,'Exact normal-exit plan required')
        with self.up.lock():
            require(self.plan()==p,'Owner, window, work or profile changed')
            gui=Observer(p['gui'])
            try:bg=Observer(p['background'])
            except BaseException:gui.release();raise
            try:
                r={'schema':1,'kind':KIND,'plan':p,'plan_sha256':digest,'phase':'gui-close-intent'};v=atomic_record(self.record,r,None)
                require(self.configuration()==p['configuration'],'Startup changed before window close')
                gui.verify();close(p['window']);gui.wait()
                require(not self.n.frontend_busy(),'Frontend lock remains; no background interrupt')
                require(self.up.invariants()==p['invariants'],'Original startup/profile changed')
                require(self.n.owners(SHA)=={p['background']['pid']},'Unknown replacement owner; no interrupt')
                require(self.configuration()==p['configuration'],'Startup changed before background interrupt')
                r['phase']='background-interrupt-intent';v=atomic_record(self.record,r,v)
                bg.interrupt();bg.wait()
                end=time.monotonic()+10
                while not self.n.stopped_state():
                    require(time.monotonic()<end,'Original supervisor stop unconfirmed; no repeated signal');time.sleep(.1)
                require(not (self.paths['profile']/'background.sock').exists(),'Background endpoint remains or was replaced')
                with up.stopped_locks(self.paths['profile']):
                    require(self.up.invariants()==p['invariants'],'Original startup/profile changed')
                r['phase']='completed';atomic_record(self.record,r,v)
                return {'phase':'completed','normal_window_close':True,'normal_background_interrupt':'SIGINT',
                        'both_exact_process_exits_observed':True,'profile_preserved':True,'idle_only_guarantee':False}
            finally:gui.release();bg.release()
    def status(self):
        r,sha=read_record(self.record)
        require(r['kind']==KIND and r['schema']==1 and gate.digest(gate.canonical(r['plan']))==r['plan_sha256']
                and r['plan']['home']==str(self.up.home),'Unknown normal-exit record retained')
        owners={}
        for role in ('gui','background'):
            i=r['plan'][role]
            try:owners[role]={'original_identity_present':target(i['pid'],Path(i['executable']),i['executable_sha256'])==i}
            except (FileNotFoundError,ProcessLookupError):owners[role]={'original_identity_present':False,'observation':'absent'}
            except (OSError,ValueError,gate.IdentityRejected):owners[role]={'original_identity_present':None,'observation':'unconfirmed'}
        return {'phase':r['phase'],'record_sha256':sha,'owners':owners,'forward_replay_supported':False}

def main():
    require(sys.platform.startswith('linux') and os.getuid()!=0,'Native ordinary-user Linux required')
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--apply',action='store_true');p.add_argument('--status',action='store_true');p.add_argument('--expected-plan-sha256')
    a=p.parse_args();require(not(a.apply and a.status),'One normal-exit action required');q=Quit(Path.home())
    if a.status:r=q.status()
    else:
        plan=q.plan();digest=gate.digest(gate.canonical(plan));r=q.apply(plan,a.expected_plan_sha256) if a.apply else {'plan':plan,'plan_sha256':digest,'applied':False}
    print(json.dumps(r,indent=2))
if __name__=='__main__':main()
