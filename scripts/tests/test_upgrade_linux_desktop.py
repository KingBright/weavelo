"""Owned synthetic Desktop apps; never touch a live user profile or supervisor."""
import copy
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import upgrade_linux_desktop as up
import upgrade_macos_desktop as mac

class UpgradeTests(unittest.TestCase):
    def setUp(self):
        if not sys.platform.startswith("linux"):
            # Storage-state fixtures only; native Linux xattrs remain a separate gate.
            for name,value in [("listxattr",lambda _p:[]),("getxattr",lambda *_a:b"")]:
                mock=patch.object(up.os,name,value,create=True);mock.start();self.addCleanup(mock.stop)
        self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup)
        self.home=Path(self.tmp.name).resolve()/'home';self.home.mkdir(mode=0o700)
        self.paths=up.fresh.paths(self.home)
        for key in ('app','profile','work'):
            self.paths[key].mkdir(parents=True,mode=0o700)
        (self.paths['profile']/'gui-state').mkdir(mode=0o700)
        for name,raw in [('mesh.conf',b'synthetic public profile'),('mesh.secret',b'synthetic credential'),
                         ('background-service.enabled',b''),('desktop-instance.lock',b''),('gui-state/desktop-instance.lock',b'')]:
            p=self.paths['profile']/name;p.write_bytes(raw);p.chmod(0o600)
        self.before_profile=up.profile_pin(self.paths['profile'])
        self.new={name:('new exe' if name=='weavelo' else name).encode() for name in up.fresh.PAYLOAD}
        for name in up.fresh.PAYLOAD:
            p=self.paths['app']/name;p.parent.mkdir(parents=True,exist_ok=True)
            p.write_bytes(b'old exe' if name=='weavelo' else self.new[name]);p.chmod(0o755 if name=='weavelo' else 0o644)
        self.old=up.image(self.paths['app']);self.old_sha=up.gate.hash_file(self.paths['app']/'weavelo')
        self.archive=self.home/'candidate.zip'
        with zipfile.ZipFile(self.archive,'w') as archive:
            for name,raw in self.new.items():archive.writestr('Weavelo/'+name,raw)
        self.files=[{'path':n,'bytes':len(v),'sha256':up.gate.digest(v)} for n,v in sorted(self.new.items())]
        self.manifest={'binary_sha256':up.gate.digest(self.new['weavelo']),'files':self.files}
        self.pins=['a'*64,'b'*64,'c'*40,'d'*64]
        self.entries={k:(('existing-'+k).encode(),0o644) for k in ('launcher','service','desktop','icon')}
        for k,(raw,mode) in self.entries.items():
            p=self.paths[k];p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(raw);p.chmod(mode)
        first={'kind':'weavelo-independent-linux-first-install','paths':{k:str(v) for k,v in self.paths.items()},'binary_sha256':self.old_sha}
        journal={'phase':'installed-not-started','plan':first,'plan_sha256':up.fresh.plan_sha(first),'user_note':'preserve'}
        p=self.paths['work']/'state.json';p.write_text(json.dumps(journal));p.chmod(0o600);self.original=p.read_bytes()
        owner=self
        class Control:
            running=set()
            def configuration(self):return {'enabled':'enabled','dropins':{},'active':'inactive','pid':0}
            def observe(self,sha):return copy.deepcopy(owner.runtime)
            def stop_gui(self,*args):
                owner.calls.append('gui-stop')
                if owner.fail=='gui-stop':raise TimeoutError('synthetic unknown stop')
            def stop_background(self,*args):
                owner.calls.append('background-stop')
                if owner.fail=='background-stop':raise TimeoutError('synthetic unknown stop')
            def owners(self,*args):return self.running
            def start(self,before,sha):
                owner.calls.append('start')
                if owner.fail=='start':raise TimeoutError('synthetic unknown activation')
                owner.runtime=copy.deepcopy(before)
                return copy.deepcopy(owner.runtime)
        self.control=Control();self.runtime={'background':{'pid':101,'generation':'101-1'},'gui':{'pid':102,'generation':'102-1','supported':True},'blockers':[],'active':True}
        self.calls=[];self.fail=None
        self.up=up.Upgrade(self.home,self.control)
        for target,name,value in [(up.fresh,'verify',lambda *args:self.manifest),(up.fresh,'payloads',lambda *args:self.entries),
                                  (up,'exchange',mac.exchange if sys.platform=='darwin' else up.exchange)]:
            context=patch.object(target,name,value);context.start();self.addCleanup(context.stop)
    def plan(self):
        p=self.up.plan(self.archive,self.pins);return p,up.gate.digest(up.gate.canonical(p))
    def unchanged(self):
        self.assertEqual((self.paths['work']/'state.json').read_bytes(),self.original)
        self.assertEqual(up.profile_pin(self.paths['profile']),self.before_profile)
    def test_success_retains_exact_old_inode_tree_and_original_membership_startup(self):
        plan,digest=self.plan();r=self.up.apply(plan,digest)
        self.assertEqual(r['phase'],'completed');self.assertEqual(self.calls,['gui-stop','background-stop','start'])
        self.assertEqual(up.image(self.up.slot),self.old);self.assertTrue(self.up.status()['rollback_tuple_verified']);self.unchanged()
        r=self.up.recover(r['transaction_id']);self.assertEqual(r['phase'],'rolled-back')
        self.assertEqual(up.image(self.paths['app']),self.old);self.unchanged()
    def test_old_unsupported_gui_or_background_blocks_before_any_transaction_or_stop(self):
        for reason in ['old GUI normal quit required','old background idle-only stop unsupported']:
            self.runtime['blockers']=[reason];plan,digest=self.plan()
            self.assertFalse(plan['applicable'])
            with self.assertRaises(ValueError):self.up.apply(plan,digest)
            self.assertFalse(self.up.transaction.exists());self.assertEqual(self.calls,[]);self.unchanged()
    def test_active_or_pending_runtime_changes_refuse_before_stop(self):
        plan,digest=self.plan();self.runtime['gui']['generation']='replacement'
        with self.assertRaises(ValueError):self.up.apply(plan,digest)
        self.assertEqual(self.calls,[]);self.assertFalse(self.up.transaction.exists())
    def check_timeout(self,stage):
        self.fail=stage;plan,digest=self.plan()
        with self.assertRaises(TimeoutError):self.up.apply(plan,digest)
        before=list(self.calls);status=self.up.status()
        with self.assertRaises(ValueError):self.up.apply(plan,digest)
        self.assertEqual(self.calls,before);self.unchanged()
        self.fail=None;self.up.recover(status['transaction_id'])
        self.assertEqual(up.image(self.paths['app']),self.old)
    def test_gui_stop_timeout_is_not_replayed(self):self.check_timeout('gui-stop')
    def test_background_stop_timeout_is_not_replayed(self):self.check_timeout('background-stop')
    def test_unknown_activation_is_not_replayed(self):self.check_timeout('start')
    def test_unknown_stop_cannot_recover_while_a_live_owner_remains(self):
        self.fail='gui-stop';plan,digest=self.plan()
        with self.assertRaises(TimeoutError):self.up.apply(plan,digest)
        status=self.up.status();self.control.running={102}
        with self.assertRaises(ValueError):self.up.recover(status['transaction_id'])
        self.assertEqual(self.calls,['gui-stop']);self.assertEqual(up.image(self.paths['app']),self.old)
    def test_exchange_unknown_is_classified_then_only_explicitly_rolled_back(self):
        plan,digest=self.plan();real=up.exchange
        def interrupted(a,b):real(a,b);raise OSError('synthetic exchange acknowledgement lost')
        with patch.object(up,'exchange',interrupted),self.assertRaises(OSError):self.up.apply(plan,digest)
        status=self.up.status();self.assertEqual(status['live_side'],'new');self.assertEqual(status['phase'],'exchange-intent')
        self.up.recover(status['transaction_id']);self.assertEqual(up.image(self.paths['app']),self.old);self.unchanged()
    def check_phase_interruption(self,phase):
        plan,digest=self.plan();save=self.up.save
        def interrupted(record,expected):
            result=save(record,expected)
            if record['phase']==phase:raise RuntimeError('synthetic interruption')
            return result
        with patch.object(self.up,'save',interrupted),self.assertRaises(RuntimeError):self.up.apply(plan,digest)
        status=self.up.status();self.up.recover(status['transaction_id'])
        self.assertEqual(up.image(self.paths['app']),self.old);self.unchanged()
    def test_unknown_backup_or_startup_drift_refuses_restore_without_overwrite(self):
        plan,digest=self.plan();r=self.up.apply(plan,digest)
        (self.up.slot/'weavelo').write_bytes(b'unknown app')
        with self.assertRaises(ValueError):self.up.recover(r['transaction_id'])
        self.assertEqual((self.up.slot/'weavelo').read_bytes(),b'unknown app')
    def test_complete_metadata_pending_after_fsync_is_recoverable_only_backwards(self):
        plan,digest=self.plan();replace=os.replace
        def interrupt(source,destination):
            if Path(destination)==self.up.journal and json.loads(Path(source).read_text())['phase']=='exchange-intent':
                raise OSError('synthetic after fsync before replace')
            return replace(source,destination)
        with patch.object(up.os,'replace',interrupt),self.assertRaises(OSError):self.up.apply(plan,digest)
        status=self.up.status();self.assertTrue(status['pending_metadata']);self.assertEqual(status['live_side'],'old')
        self.up.recover(status['transaction_id']);self.assertFalse(self.up.status()['pending_metadata']);self.unchanged()
    def test_partial_metadata_pending_is_retained_without_recovery_or_replay(self):
        plan,digest=self.plan();save=self.up.save
        def stop(record,expected):
            result=save(record,expected)
            if record['phase']=='staged':raise RuntimeError('synthetic interruption')
            return result
        with patch.object(self.up,'save',stop),self.assertRaises(RuntimeError):self.up.apply(plan,digest)
        record,_=up.read_record(self.up.journal)
        pending=self.up.journal.with_name('state.json.upgrade-tmp');pending.write_bytes(b'{unknown partial');pending.chmod(0o600)
        with self.assertRaises(ValueError):self.up.recover(record['transaction_id'])
        self.assertEqual(pending.read_bytes(),b'{unknown partial');self.assertEqual(up.image(self.paths['app']),self.old);self.unchanged()
    def test_partial_candidate_is_retained_and_old_image_can_be_restored_without_replay(self):
        plan,digest=self.plan();write=up.fresh.write_new
        def interrupt(path,*args):
            write(path,*args);raise OSError('synthetic candidate write interrupted')
        with patch.object(up.fresh,'write_new',interrupt),self.assertRaises(OSError):self.up.apply(plan,digest)
        status=self.up.status();self.assertEqual(status['slot_state'],'partial-or-unknown-retained')
        before=list(self.up.slot.rglob('*'));self.up.recover(status['transaction_id'])
        self.assertEqual(list(self.up.slot.rglob('*')),before);self.unchanged()
    def test_unknown_complete_pending_fields_phase_or_candidate_are_retained(self):
        plan,digest=self.plan();save=self.up.save
        def stop(record,expected):
            result=save(record,expected)
            if record['phase']=='staged':raise RuntimeError('synthetic interruption')
            return result
        with patch.object(self.up,'save',stop),self.assertRaises(RuntimeError):self.up.apply(plan,digest)
        record,_=up.read_record(self.up.journal)
        pending=self.up.journal.with_name('state.json.upgrade-tmp')
        for change in ['phase','extra','candidate']:
            value=copy.deepcopy(record);value['phase']='gui-stop-intent'
            if change=='phase':value['phase']='unknown'
            if change=='extra':value['unknown_field']=True
            if change=='candidate':value['candidate_image']['files'][0]['sha256']='0'*64
            raw=(json.dumps(value,indent=2)+'\n').encode();pending.write_bytes(raw);pending.chmod(0o600)
            with self.assertRaises(ValueError):self.up.recover(record['transaction_id'])
            self.assertEqual(pending.read_bytes(),raw);self.assertEqual(self.calls,[])
            self.assertEqual(up.image(self.paths['app']),self.old)
        self.unchanged()
    def test_loaded_exec_metadata_cannot_substitute_another_program_or_arguments(self):
        exe=self.paths['app']/'weavelo'
        value={'type':'a(sasbttttuii)','data':[[str(exe),[str(exe),'--background-service'],False,*([0]*7)]]}
        self.assertEqual(up.loaded_exec(value,exe)['arguments'],['--background-service'])
        for mutate in ['program','args','ignore','additional']:
            bad=copy.deepcopy(value)
            if mutate=='program':bad['data'][0][0]='/bin/sh'
            if mutate=='args':bad['data'][0][1].append('unreviewed')
            if mutate=='ignore':bad['data'][0][2]=True
            if mutate=='additional':bad['data'].append(copy.deepcopy(bad['data'][0]))
            with self.assertRaises(ValueError):up.loaded_exec(bad,exe)
    def test_historical_completed_pending_allows_backward_recovery_after_normal_owner_exit(self):
        plan,digest=self.plan();replace=os.replace
        def interrupt(source,destination):
            if Path(destination)==self.up.journal and json.loads(Path(source).read_text())['phase']=='completed':
                raise OSError('synthetic completed fsync acknowledgement lost')
            return replace(source,destination)
        with patch.object(up.os,'replace',interrupt),self.assertRaises(OSError):self.up.apply(plan,digest)
        status=self.up.status();self.runtime={'background':None,'gui':None,'blockers':[],'active':False}
        calls=self.calls.count('start')
        self.up.recover(status['transaction_id'])
        self.assertEqual(self.calls.count('start'),calls+1)  # old app restoration only
        self.assertEqual(up.image(self.paths['app']),self.old);self.unchanged()
    def check_terminal_rollback(self,still_running):
        plan,digest=self.plan();result=self.up.apply(plan,digest);replace=os.replace
        def interrupt(source,destination):
            if Path(destination)==self.up.journal and json.loads(Path(source).read_text())['phase']=='rolled-back':
                raise OSError('synthetic terminal rollback checkpoint interrupted')
            return replace(source,destination)
        with patch.object(up.os,'replace',interrupt),self.assertRaises(OSError):self.up.recover(result['transaction_id'])
        self.control.running={101} if still_running else set()
        if not still_running:self.runtime={'background':None,'gui':None,'blockers':[],'active':False}
        before=list(self.calls)
        final=self.up.recover(result['transaction_id'])
        self.assertTrue(final['terminal_checkpoint_confirmed']);self.assertFalse(final['activation_replayed'])
        self.assertEqual(self.calls,before);self.assertEqual(self.up.status()['phase'],'rolled-back');self.unchanged()
        self.up.recover(result['transaction_id']);self.assertEqual(self.calls,before)
    def test_terminal_rollback_checkpoint_with_live_old_owner_never_restarts(self):self.check_terminal_rollback(True)
    def test_terminal_rollback_checkpoint_after_normal_quit_never_restarts(self):self.check_terminal_rollback(False)
    def test_candidate_security_label_drift_refuses_before_any_owner_stop(self):
        plan,digest=self.plan();real=up.security_labels
        def changed(value):
            result=real(value)
            if value['files']==self.files:result['directories']['.']['security.selinux']='different-label'
            return result
        with patch.object(up,'security_labels',changed),self.assertRaises(ValueError):self.up.apply(plan,digest)
        self.assertEqual(self.calls,[]);self.assertEqual(up.image(self.paths['app']),self.old);self.unchanged()
    def test_app_mount_point_refuses_before_any_owner_stop(self):
        with patch.object(up.os.path,'ismount',return_value=True),self.assertRaises(ValueError):up.Upgrade(self.home,self.control)
        self.assertEqual(self.calls,[]);self.unchanged()
    def test_app_device_boundary_is_checked_not_only_its_parent(self):
        original=Path.stat;app=self.paths['app']
        def stat_result(path,*args,**kwargs):
            result=original(path,*args,**kwargs)
            if path==app:
                values=list(result);values[2]+=1;return os.stat_result(values)
            return result
        with patch.object(Path,'stat',stat_result),self.assertRaises(ValueError):up.Upgrade(self.home,self.control)
        self.assertEqual(self.calls,[]);self.unchanged()
    def test_same_device_work_bind_mount_refuses_before_any_stop(self):
        original=Path.read_text;work=self.paths['work']
        text='1 0 0:1 / / rw - fixture fixture rw\n'+f'2 1 0:1 / {work} rw - fixture fixture rw\n'
        def read(path,*args,**kwargs):
            return text if str(path)=='/proc/self/mountinfo' else original(path,*args,**kwargs)
        with patch.object(up.sys,'platform','linux'),patch.object(Path,'read_text',read),self.assertRaises(ValueError):
            up.Upgrade(self.home,self.control)
        self.assertEqual(self.calls,[]);self.unchanged()
    def test_loaded_unit_cache_or_extra_hooks_refuse_before_start(self):
        from types import SimpleNamespace
        native=up.Native.__new__(up.Native);native.paths=self.paths;native.exe=self.paths['app']/'weavelo'
        fields={k:'' for k in ['MainPID','ActiveState','SubState','UnitFileState','FragmentPath','DropInPaths','Restart','Job',
             'TriggeredBy','RestartForceExitStatus','ExecStop','ExecStopPost','OnSuccess','CPUWeight','TimeoutStopFailureMode',
             'NeedDaemonReload','ExecStartPre','ExecStartPost','ExecCondition']}
        fields.update(MainPID='101',ActiveState='active',SubState='running',UnitFileState='enabled',
                      FragmentPath=str(self.paths['service']),Restart='on-failure',NeedDaemonReload='no')
        for key,value in [('NeedDaemonReload','yes'),('ExecStartPre','unknown hook'),('ExecCondition','unknown condition')]:
            bad=dict(fields);bad[key]=value
            result=SimpleNamespace(returncode=0,stdout='\n'.join(k+'='+v for k,v in bad.items()))
            with patch.object(up.subprocess,'run',return_value=result),self.assertRaises(ValueError):native.configuration()
    def native_fixture(self):
        native=up.Native.__new__(up.Native);native.paths=self.paths;native.exe=self.paths['app']/'weavelo'
        native.configuration=lambda:{'pid':101,'active':'active','substate':'running'}
        native.frontend_busy=lambda:True
        def query(flag):
            if flag=='--gui-status-json':return {'ok':True,'renderer':up.gate.RENDERER,'generation':'102-1','executable_sha256':'a'*64}
            if flag=='--gui-network-diagnostics-json':return {'ok':True,'generation':'102-1','executable_sha256':'a'*64,'pid':102}
            raise ValueError('unexpected synthetic query')
        native.query=query
        return native
    def test_only_registered_profile_pids_are_read_not_unrelated_protected_proc(self):
        native=self.native_fixture()
        with patch.object(Path,'iterdir',side_effect=PermissionError('unrelated proc')),patch.object(up.gate,'verify_running_linux') as verify:
            self.assertEqual(native.owners('a'*64),{101,102})
        self.assertEqual([call.args[0] for call in verify.call_args_list],[101,102])
    def test_known_target_process_permission_failure_is_never_ignored(self):
        native=self.native_fixture()
        with patch.object(up.gate,'verify_running_linux',side_effect=[None,PermissionError('target proc')]),self.assertRaises(PermissionError):
            native.owners('a'*64)
    def test_zero_mainpid_with_any_canonical_profile_lock_busy_is_not_stopped(self):
        import fcntl
        native=self.native_fixture();native.configuration=lambda:{'pid':0,'active':'inactive','substate':'dead'}
        for path in (self.paths['profile'],self.paths['profile']/'desktop-instance.lock',self.paths['profile']/'gui-state/desktop-instance.lock'):
            fd=os.open(path,os.O_RDONLY)
            try:
                fcntl.flock(fd,fcntl.LOCK_EX|fcntl.LOCK_NB)
                with self.assertRaises(BlockingIOError):native.owners('a'*64)
            finally:os.close(fd)
        self.assertEqual(native.owners('a'*64),set())
    def test_busy_frontend_missing_or_replaced_private_status_refuses(self):
        native=self.native_fixture();query=native.query
        def replaced(flag):
            value=query(flag)
            if flag=='--gui-network-diagnostics-json':value['generation']='103-2';value['pid']=103
            return value
        with patch.object(up.gate,'verify_running_linux'):
            native.query=replaced
            with self.assertRaises(ValueError):native.owners('a'*64)
            native.query=lambda *_:(_ for _ in ()).throw(ValueError('missing endpoint'))
            with self.assertRaises(ValueError):native.owners('a'*64)
    def test_supervisor_owner_replacement_during_observation_refuses(self):
        native=self.native_fixture();native.owners=lambda _:{101}
        native.query=lambda *_:{'role':'background-service','pid':103,'executable_sha256':'a'*64,'profile':str(self.paths['profile'])}
        with self.assertRaises(ValueError):native.observe('a'*64)
    def test_activation_transitional_reads_do_not_repeat_the_start_operation(self):
        from unittest.mock import Mock
        native=self.native_fixture();before={'background':{'pid':101,'generation':'101-1'},'gui':None,'active':True,'blockers':[]}
        native.ctl=Mock();native.owners=Mock(side_effect=[PermissionError('not yet inspectable'),{101}])
        native.query=lambda *_:{'role':'background-service','pid':101,'executable_sha256':'a'*64,'profile':str(self.paths['profile'])}
        native.observe=lambda _:before
        with patch.object(up.time,'sleep'):self.assertEqual(native.start(before,'a'*64),before)
        native.ctl.assert_called_once_with('start')
    def missing_exec_fixture(self,document=None):
        from types import SimpleNamespace
        native=self.native_fixture()
        keys=['MainPID','ActiveState','SubState','UnitFileState','FragmentPath','DropInPaths','Restart','Job',
             'TriggeredBy','RestartForceExitStatus','OnSuccess','CPUWeight','TimeoutStopFailureMode','NeedDaemonReload']
        fields={key:'' for key in keys}
        fields.update(MainPID='101',ActiveState='active',SubState='running',UnitFileState='enabled',
                      FragmentPath=str(self.paths['service']),Restart='on-failure',NeedDaemonReload='no')
        exe=self.paths['app']/'weavelo'
        def run(argv,**kwargs):
            self.assertNotIn('Environment',argv);self.assertNotIn('EnvironmentFiles',argv)
            if argv[0]=='systemctl':return SimpleNamespace(returncode=0,stdout='\n'.join(k+'='+v for k,v in fields.items()))
            value=({'type':'a(sasbttttuii)','data':[[str(exe),[str(exe),'--background-service'],False,*([0]*7)]]}
                if argv[-1]=='ExecStart' else (document if document is not None else {'type':'a(sasbttttuii)','data':[]}))
            return SimpleNamespace(returncode=0,stdout=json.dumps(value))
        # Exercise real Native.configuration/exec_property instead of the fixture
        # lookup methods, with exactly the observed deprecated-field shape.
        native.configuration=lambda:up.Native.configuration(native)
        return native,run,fields
    def test_missing_deprecated_exec_fields_require_verified_empty_bus_properties(self):
        native,run,_=self.missing_exec_fixture()
        with patch.object(up.subprocess,'run',side_effect=run):
            result=native.configuration()
        for key in up.EXEC_HOOKS:self.assertEqual(result['policy'][key],'')
    def test_nonempty_or_unknown_typed_exec_hook_is_never_normalized_empty(self):
        for document in [{'type':'a(sasbttttuii)','data':[['synthetic-hook']]},{'type':'unknown','data':[]}]:
            native,run,_=self.missing_exec_fixture(document)
            with patch.object(up.subprocess,'run',side_effect=run),self.assertRaises(ValueError):native.configuration()
        native,run,fields=self.missing_exec_fixture();fields.pop('NeedDaemonReload')
        with patch.object(up.subprocess,'run',side_effect=run),self.assertRaises(ValueError):native.configuration()
    def test_execution_metadata_reader_refuses_environment_property_requests(self):
        native=self.native_fixture()
        with patch.object(up.subprocess,'run') as run,self.assertRaises(ValueError):native.exec_property('Environment')
        run.assert_not_called()
    def test_pending_protected_recovery_and_concurrent_installer_refuse(self):
        import fcntl
        fd=os.open(self.paths['work'],os.O_RDONLY|os.O_DIRECTORY)
        try:
            fcntl.flock(fd,fcntl.LOCK_EX|fcntl.LOCK_NB)
            plan,digest=self.plan()
            with self.assertRaises(BlockingIOError):self.up.apply(plan,digest)
        finally:os.close(fd)
        (self.paths['profile']/'mesh.secret-recovery').symlink_to('missing')
        with self.assertRaises(ValueError):self.plan()


# Each boundary gets an independent disposable installation/transaction.
for _phase in ['preparing','staged','gui-stop-intent','background-stop-intent','owners-stopped','exchange-intent','exchanged','activation-intent','completed']:
    def _case(self,phase=_phase):self.check_phase_interruption(phase)
    setattr(UpgradeTests,'test_interrupt_'+_phase.replace('-','_'),_case)
