"""Owned disposable upgrade fixtures. No live service, signing or permission changes."""
import contextlib
import copy
import fcntl
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import sys
import subprocess
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import upgrade_macos_desktop as upgrade
NativeControlOriginal = upgrade.NativeControl
import desktop_identity as desktop
import macos_release_guard as guard


class UpgradeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name).resolve() / 'home'; self.home.mkdir(mode=0o700)
        self.selected = desktop.paths(self.home)
        for path in (self.selected['app'], self.selected['profile'], self.selected['state'],
                     self.selected['plist'].parent):
            path.mkdir(mode=0o700, parents=True, exist_ok=True)
        self.old_app = self.home / 'Applications/Weavelo.app'
        self.old_app.mkdir(); (self.old_app/'sentinel').write_bytes(b'retained-client')
        self.candidate = self.home/'candidate/Weavelo.app'
        self.make_app(self.selected['app'], b'old-image')
        self.make_app(self.candidate, b'new-image')
        for name in ('mesh.conf', 'mesh.secret'):
            path = self.selected['profile']/name
            path.write_bytes(b'synthetic-private-fixture'); path.chmod(0o600)
        self.selected['plist'].write_bytes(plistlib.dumps(desktop.launch_agent(self.home)))
        self.selected['plist'].chmod(0o600)
        self.registry = self.selected['state']/'install.json'
        self.record = {'application_id': desktop.identity()['application_id'], 'applied': True,
                       'phase': 'committed_not_started', 'paths': {k:str(v) for k,v in self.selected.items()},
                       'source': {'commit':'a'*40, 'identity_sha256':'b'*64, 'manifest_sha256':'c'*64},
                       'user_notes': {'keep': 'user content'}}
        self.registry.write_text(json.dumps(self.record)); self.registry.chmod(0o600)
        self.archive = self.home/'candidate.zip'; self.archive.write_bytes(b'synthetic-archive')
        self.pins = {'commit':'d'*40,'identity_sha256':'e'*64,'manifest_sha256':'f'*64}
        self.before_hook = self.stop_hook = self.start_hook = None
        self.calls = []
        owner = self
        class Control:
            def __init__(self, selected): self.selected=selected
            def before(self, expected):
                owner.calls.append('before')
                if owner.before_hook: owner.before_hook()
                return {'background': {'pid':101}, 'gui':{'pid':102}, 'loaded':True}
            def stop(self, before):
                owner.calls.append('stop')
                if owner.stop_hook: owner.stop_hook()
            def start(self, before, expected):
                owner.calls.append('start')
                if owner.start_hook: owner.start_hook()
                return {'background_pid':201,'frontend_pid':202,'native_pairing_cli_verified':True,
                        'executable_sha256':expected}
        self.control = Control
        for obj,name,value in ((guard,'verify_app',self.signed),
                               (guard,'verified_archive',self.archive_view),
                               (upgrade.fresh,'verified_product',lambda *a: {'desktop_identity':desktop.identity()}),
                               (upgrade,'product_current',lambda *a: {'desktop_identity':desktop.identity()}),
                               (upgrade,'NativeControl',Control)):
            p=patch.object(obj,name,value);p.start();self.addCleanup(p.stop)

    def make_app(self, path, value):
        (path/'Contents/MacOS').mkdir(parents=True)
        (path/'Contents/MacOS/weavelo').write_bytes(value)
        (path/'Contents/Info.plist').write_bytes(value+b'-info')

    def signed(self, path, policy, **kw):
        image=(path/'Contents/MacOS/weavelo').read_bytes()
        return {'version':'2.0.0-alpha.9','build':'20261022.10' if image==b'new-image' else '20261022.9',
                'bundle_id':policy.bundle_id,'certificate_sha1':policy.certificate_sha1,
                'designated_requirement':policy.requirement,
                'executable_sha256':hashlib.sha256(image).hexdigest(),
                'info_sha256':hashlib.sha256(image+b'-info').hexdigest(),'signature_verified':True}

    @contextlib.contextmanager
    def archive_view(self,*args,**kw):
        yield self.candidate,self.signed(self.candidate,guard.load_policy(guard.DESKTOP_POLICY_FILE))

    def current(self):
        return upgrade.inspect(self.home)['pins']

    def apply(self, pins=None, resume=None, no_new=False):
        return upgrade.apply(self.archive,self.home,self.pins,upgrade.gate.hash_file(self.archive),
                             self.current() if pins is None else pins, resume_sha=resume,
                             no_new_rollback=no_new)

    def app_bytes(self):
        return (self.selected['app']/'Contents/MacOS/weavelo').read_bytes()

    def test_readonly_plan_preserves_membership_launch_policy_and_user_content(self):
        before=(self.registry.read_bytes(),self.selected['plist'].read_bytes(),
                upgrade.membership_metadata(self.selected['profile']))
        self.assertEqual(upgrade.inspect(self.home)['record']['user_notes'],self.record['user_notes'])
        self.assertEqual(before,(self.registry.read_bytes(),self.selected['plist'].read_bytes(),
                                upgrade.membership_metadata(self.selected['profile'])))
        self.assertEqual(self.calls,[])

    def test_each_current_version_pin_conflict_refuses_before_stop(self):
        for key in ('executable','install','plist'):
            pins=self.current();pins[key]='1'*64
            with self.assertRaisesRegex(guard.ReleaseRejected,'version conflict'):self.apply(pins)
        self.assertEqual(self.calls,[]);self.assertEqual(self.app_bytes(),b'old-image')

    def test_native_gui_restart_keeps_current_foreground_application(self):
        control = NativeControlOriginal(self.selected)
        digest = self.current()['executable']
        replies = {'--background-status-json': {'role': 'background-service', 'pid': 201,
                                               'executable_sha256': digest},
                   '--pairing-status-json': {'schema': 1, 'accepted': True},
                   '--gui-network-diagnostics-json': {'ok': True, 'pid': 202,
                                                     'executable_sha256': digest}}
        with patch.object(control, 'loaded', return_value=True), \
             patch.object(control, 'query', side_effect=lambda flag, *args: replies[flag]), \
             patch.object(upgrade.subprocess, 'run') as run:
            result = control.start({'background': None, 'gui': {'pid': 102}}, digest)
        opened = [call.args[0] for call in run.call_args_list if call.args[0][0] == '/usr/bin/open']
        self.assertEqual(opened, [['/usr/bin/open', '-g', '-n', str(self.selected['app']), '--args', '--gui']])
        self.assertEqual(result['frontend_pid'], 202)

    def test_bad_archive_digest_refuses_before_destination_write(self):
        with self.assertRaisesRegex(guard.ReleaseRejected,'digest'):
            upgrade.apply(self.archive,self.home,self.pins,'1'*64,self.current())
        self.assertFalse((self.selected['state']/'upgrade.json').exists());self.assertFalse(self.calls)

    def test_bad_signature_and_bad_source_each_refuse_before_stop(self):
        with patch.object(guard,'verified_archive',side_effect=guard.ReleaseRejected('signature')):
            with self.assertRaises(guard.ReleaseRejected):self.apply()
        with patch.object(upgrade.fresh,'verified_product',side_effect=guard.ReleaseRejected('source')):
            with self.assertRaises(guard.ReleaseRejected):self.apply()
        self.assertFalse(self.calls)

    def test_no_new_rollback_does_not_bypass_signature_or_source_checks(self):
        with patch.object(guard,'verified_archive',side_effect=guard.ReleaseRejected('signature')):
            with self.assertRaises(guard.ReleaseRejected):self.apply(no_new=True)
        with patch.object(upgrade.fresh,'verified_product',side_effect=guard.ReleaseRejected('source')):
            with self.assertRaises(guard.ReleaseRejected):self.apply(no_new=True)
        self.assertFalse(self.calls)
        self.assertFalse((self.selected['state']/'upgrade.json').exists())

    def test_same_build_different_bytes_and_signer_change_and_downgrade_are_rejected(self):
        before=self.signed(self.selected['app'],guard.load_policy(guard.DESKTOP_POLICY_FILE))
        for change in ({'build':before['build']},{'certificate_sha1':'0'*40},{'version':'2.0.0-alpha.8'}):
            candidate=dict(self.signed(self.candidate,guard.load_policy(guard.DESKTOP_POLICY_FILE)),**change)
            @contextlib.contextmanager
            def view(*a,**k):yield self.candidate,candidate
            with patch.object(guard,'verified_archive',view):
                with self.assertRaises(guard.ReleaseRejected):self.apply()
        self.assertFalse(self.calls)

    def test_incomplete_journal_or_existing_stage_is_never_replayed(self):
        journal=self.selected['state']/'upgrade.json';journal.write_text(json.dumps({'schema':1,'phase':'exchanged'}));journal.chmod(0o600)
        with self.assertRaisesRegex(guard.ReleaseRejected,'incomplete'):self.apply({})
        journal.unlink()
        stage=self.selected['app'].parent/'.desktop-upgrade.pending';stage.mkdir()
        pins=self.current()
        with self.assertRaisesRegex(guard.ReleaseRejected,'staging'):self.apply(pins)
        self.assertFalse(self.calls);self.assertEqual(self.app_bytes(),b'old-image')

    def prestop_stage(self):
        current = self.current()
        stage = self.selected['app'].parent / '.desktop-upgrade.pending'
        stage.mkdir(mode=0o700)
        shutil.copytree(self.candidate, stage / 'Weavelo.app')
        value = {'schema': 1, 'kind': 'independent-desktop-upgrade', 'phase': 'staged',
                 'source': self.pins, 'archive_sha256': upgrade.gate.hash_file(self.archive),
                 'current': current, 'staging': str(stage), 'retained_client_modified': False}
        journal = self.selected['state'] / 'upgrade.json'
        raw = (json.dumps(value, indent=2) + '\n').encode()
        journal.write_bytes(raw); journal.chmod(0o600)
        return current, upgrade.gate.hash_file(journal), raw

    def test_retirement_plan_has_no_writes_or_process_control(self):
        pins, digest, raw = self.prestop_stage()
        result = upgrade.retire_staged(self.home, pins, digest)
        self.assertFalse(result['applied'])
        self.assertEqual((self.selected['state'] / 'upgrade.json').read_bytes(), raw)
        self.assertFalse(upgrade.retirement_root(self.selected).exists())
        self.assertEqual(self.calls, [])

    def test_retirement_keeps_exact_history_stage_current_backup_and_old_slot(self):
        root = self.selected['app'].parent / '.desktop-upgrade.rollback'
        root.mkdir(mode=0o700); (root / 'sentinel').write_bytes(b'prior rollback')
        before = upgrade.tree_digest(root)
        pins, digest, raw = self.prestop_stage()
        membership = upgrade.membership_metadata(self.selected['profile'])
        result = upgrade.retire_staged(self.home, pins, digest, apply=True)
        recovery = upgrade.retirement_root(self.selected)
        self.assertTrue(result['applied']); self.assertFalse(result['installed'])
        self.assertEqual(result['phase'], upgrade.RETIRED)
        self.assertEqual((recovery / 'original-upgrade.json').read_bytes(), raw)
        self.assertEqual((recovery / 'Weavelo.app/Contents/MacOS/weavelo').read_bytes(), b'old-image')
        self.assertEqual((recovery / 'staging/Weavelo.app/Contents/MacOS/weavelo').read_bytes(), b'new-image')
        self.assertEqual((recovery / 'install.before.json').read_bytes(), self.registry.read_bytes())
        self.assertEqual((recovery / 'launch-agent.before.plist').read_bytes(), self.selected['plist'].read_bytes())
        self.assertEqual(upgrade.tree_digest(root), before)
        self.assertEqual(upgrade.membership_metadata(self.selected['profile']), membership)
        self.assertEqual(self.current(), pins); self.assertEqual(self.app_bytes(), b'old-image')
        self.assertEqual(self.calls, [])

    def test_retirement_refuses_wrong_phase_current_or_record_pin(self):
        pins, digest, raw = self.prestop_stage()
        with self.assertRaises(guard.ReleaseRejected):
            upgrade.retire_staged(self.home, pins, '1'*64, apply=True)
        bad = dict(pins, executable='1'*64)
        with self.assertRaisesRegex(guard.ReleaseRejected, 'version conflict'):
            upgrade.retire_staged(self.home, bad, digest, apply=True)
        journal = self.selected['state'] / 'upgrade.json'
        for change in ({'phase': 'stopping'}, {'owners_before': {}}, {'rollback': {}}):
            journal.write_text(json.dumps(dict(json.loads(raw), **change)))
            with self.assertRaisesRegex(guard.ReleaseRejected, 'pre-stop'):
                upgrade.retire_staged(self.home, pins, upgrade.gate.hash_file(journal), apply=True)
        self.assertFalse(upgrade.retirement_root(self.selected).exists()); self.assertEqual(self.calls, [])

    def test_retirement_refuses_bad_stage_signature_and_unknown_slot(self):
        pins, digest, _ = self.prestop_stage()
        with patch.object(guard, 'verify_app', side_effect=guard.ReleaseRejected('signature')):
            with self.assertRaisesRegex(guard.ReleaseRejected, 'signature'):
                upgrade.retire_staged(self.home, pins, digest, apply=True)
        root = upgrade.retirement_root(self.selected); root.mkdir(mode=0o700)
        (root / 'sentinel').write_bytes(b'unknown')
        with self.assertRaises(OSError): upgrade.retire_staged(self.home, pins, digest, apply=True)
        self.assertEqual((root / 'sentinel').read_bytes(), b'unknown'); self.assertEqual(self.calls, [])

    def test_retirement_refuses_linked_slot_and_busy_installer_before_writes(self):
        pins, digest, raw = self.prestop_stage()
        root = upgrade.retirement_root(self.selected); root.symlink_to(self.home, target_is_directory=True)
        with self.assertRaisesRegex(guard.ReleaseRejected, 'Linked retirement'):
            upgrade.retire_staged(self.home, pins, digest, apply=True)
        root.unlink()
        lock = self.selected['state'].parent / '.desktop-install.lock'
        with lock.open('w') as held:
            lock.chmod(0o600); fcntl.flock(held, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaises(BlockingIOError): upgrade.retire_staged(self.home, pins, digest, apply=True)
        self.assertFalse(root.exists())
        self.assertEqual((self.selected['state'] / 'upgrade.json').read_bytes(), raw)
        self.assertEqual(self.calls, [])

    def test_retirement_recovers_exact_moved_stage_before_marker_commit(self):
        pins, digest, raw = self.prestop_stage()
        atomic = upgrade.atomic_bytes
        journal = self.selected['state'] / 'upgrade.json'
        def interrupt(path, *args, **kwargs):
            if path == journal: raise OSError('synthetic interruption before marker')
            return atomic(path, *args, **kwargs)
        with patch.object(upgrade, 'atomic_bytes', side_effect=interrupt):
            with self.assertRaisesRegex(OSError, 'synthetic'): upgrade.retire_staged(self.home, pins, digest, apply=True)
        self.assertEqual(journal.read_bytes(), raw)
        self.assertTrue((upgrade.retirement_root(self.selected) / 'staging').is_dir())
        result = upgrade.retire_staged(self.home, pins, digest, apply=True)
        self.assertEqual(result['phase'], upgrade.RETIRED); self.assertEqual(self.calls, [])

    def test_retirement_incomplete_backup_remains_inspectable(self):
        pins, digest, raw = self.prestop_stage()
        def partial(source, destination, **kwargs):
            destination.mkdir(mode=0o700); (destination / 'partial').write_bytes(b'incomplete')
        with patch.object(upgrade.shutil, 'copytree', side_effect=partial):
            with self.assertRaisesRegex(guard.ReleaseRejected, 'Incomplete retirement'):
                upgrade.retire_staged(self.home, pins, digest, apply=True)
        with self.assertRaisesRegex(guard.ReleaseRejected, 'Incomplete retirement'):
            upgrade.retire_staged(self.home, pins, digest, apply=True)
        self.assertEqual((self.selected['state'] / 'upgrade.json').read_bytes(), raw)
        self.assertTrue((self.selected['app'].parent / '.desktop-upgrade.pending').is_dir())
        self.assertEqual(self.app_bytes(), b'old-image'); self.assertEqual(self.calls, [])

    def test_forged_retirement_or_modified_evidence_never_unlocks_apply(self):
        pins, digest, raw = self.prestop_stage(); journal = self.selected['state'] / 'upgrade.json'
        journal.write_text(json.dumps(dict(json.loads(raw), phase=upgrade.RETIRED)))
        with self.assertRaises((OSError, guard.ReleaseRejected)): self.apply(pins)
        journal.write_bytes(raw)
        upgrade.retire_staged(self.home, pins, digest, apply=True)
        evidence = upgrade.retirement_root(self.selected) / 'original-upgrade.json'
        evidence.write_bytes(evidence.read_bytes() + b'\n')
        with self.assertRaisesRegex(guard.ReleaseRejected, 'evidence differs'): self.apply(pins)
        self.assertEqual(self.app_bytes(), b'old-image'); self.assertEqual(self.calls, [])

    @unittest.skipUnless(sys.platform == 'darwin', 'native atomic directory exchange')
    def test_normal_signed_upgrade_after_retirement_preserves_recovery_reference(self):
        pins, digest, raw = self.prestop_stage()
        upgrade.retire_staged(self.home, pins, digest, apply=True)
        result = self.apply(pins, no_new=True)
        self.assertEqual(result['phase'], 'completed'); self.assertEqual(self.app_bytes(), b'new-image')
        self.assertEqual(result['retired_staged']['upgrade_sha256'], digest)
        record = json.loads(self.registry.read_text())
        self.assertEqual(record['retired_staged'], result['retired_staged'])
        self.assertEqual((upgrade.retirement_root(self.selected) / 'original-upgrade.json').read_bytes(), raw)

    @unittest.skipUnless(sys.platform=='darwin','native atomic directory exchange')
    def test_native_exchange_upgrade_preserves_membership_plist_history_and_retained_client(self):
        before=(self.selected['plist'].read_bytes(),upgrade.membership_metadata(self.selected['profile']))
        result=self.apply()
        self.assertTrue(result['applied']);self.assertEqual(result['phase'],'completed')
        self.assertEqual(self.app_bytes(),b'new-image')
        self.assertEqual(before,(self.selected['plist'].read_bytes(),upgrade.membership_metadata(self.selected['profile'])))
        self.assertEqual((self.old_app/'sentinel').read_bytes(),b'retained-client')
        record=json.loads(self.registry.read_text())
        self.assertEqual(record['user_notes'],self.record['user_notes'])
        self.assertEqual(record['history'],[self.record]);self.assertEqual(record['source'],self.pins)
        self.assertEqual(self.calls,['before','stop','start'])
        self.assertFalse((self.selected['app'].parent/'.desktop-upgrade.pending').exists())
        self.assertFalse(self.apply()['applied']);self.assertEqual(self.calls,['before','stop','start'])

    def test_declined_stop_preserves_old_application_and_blocks_replay(self):
        self.stop_hook=lambda: (_ for _ in ()).throw(guard.ReleaseRejected('normal quit declined'))
        pins=self.current()
        with self.assertRaisesRegex(guard.ReleaseRejected,'declined'):self.apply(pins)
        self.assertEqual(self.app_bytes(),b'old-image')
        with self.assertRaisesRegex(guard.ReleaseRejected,'incomplete'):self.apply(pins)
        self.assertEqual(self.calls,['before','stop'])

    def test_concurrent_policy_and_membership_changes_refuse_before_exchange(self):
        def change():
            self.selected['plist'].write_bytes(self.selected['plist'].read_bytes()+b'\n')
        self.stop_hook=change;pins=self.current()
        with self.assertRaisesRegex(guard.ReleaseRejected,'version conflict'):self.apply(pins)
        self.assertEqual(self.app_bytes(),b'old-image')
        self.assertNotIn('start',self.calls)

    def test_concurrent_membership_metadata_change_is_not_repaired(self):
        def change():
            p=self.selected['profile']/'mesh.conf';s=p.stat();os.utime(p,ns=(s.st_atime_ns,s.st_mtime_ns+1000000))
        self.stop_hook=change
        with self.assertRaisesRegex(guard.ReleaseRejected,'membership changed'):self.apply()
        self.assertEqual(self.app_bytes(),b'old-image');self.assertNotIn('start',self.calls)

    @unittest.skipUnless(sys.platform=='darwin','native atomic directory exchange')
    def test_activation_failure_retains_owned_old_stage_without_automatic_rollback(self):
        self.start_hook=lambda: (_ for _ in ()).throw(guard.ReleaseRejected('activation failed'))
        pins=self.current()
        with self.assertRaisesRegex(guard.ReleaseRejected,'activation'):self.apply(pins)
        self.assertEqual(self.app_bytes(),b'new-image')
        staged=self.selected['app'].parent/'.desktop-upgrade.pending/Weavelo.app/Contents/MacOS/weavelo'
        self.assertEqual(staged.read_bytes(),b'old-image')
        self.assertEqual(json.loads(self.registry.read_text()),self.record)
        with self.assertRaisesRegex(guard.ReleaseRejected,'incomplete'):self.apply(pins)

    @unittest.skipUnless(sys.platform=='darwin','native atomic directory exchange')
    def test_concurrent_install_record_after_activation_is_preserved(self):
        def change():
            current=dict(self.record,user_notes={'new':'user edit'});self.registry.write_text(json.dumps(current))
        self.start_hook=change
        with self.assertRaisesRegex(guard.ReleaseRejected,'version conflict'):self.apply()
        self.assertEqual(json.loads(self.registry.read_text())['user_notes'],{'new':'user edit'})
        self.assertTrue((self.selected['app'].parent/'.desktop-upgrade.pending').is_dir())

    def test_redirected_or_hardlinked_install_record_and_wrong_namespace_are_refused(self):
        alias=self.registry.with_name('alias.json');os.link(self.registry,alias)
        with self.assertRaisesRegex(guard.ReleaseRejected,'Private'):upgrade.inspect(self.home)
        alias.unlink()
        data=dict(self.record,application_id='fun.hackerlife.weavelo')
        self.registry.write_text(json.dumps(data))
        with self.assertRaisesRegex(guard.ReleaseRejected,'Original'):upgrade.inspect(self.home)

    def test_unknown_history_and_overflow_never_discard_user_content(self):
        for history in ({'unexpected':'format'},['x'*64000]):
            data=dict(self.record,history=history);self.registry.write_text(json.dumps(data))
            with self.assertRaises(guard.ReleaseRejected):self.apply()
            self.assertEqual(json.loads(self.registry.read_text()),data)
        self.assertFalse(self.calls)


    def test_authenticated_gui_network_status_has_pid_and_malformed_status_is_refused(self):
        digest=self.current()['executable']
        background={'role':'background-service','pid':101,'profile':str(self.selected['profile']),
                    'executable_sha256':digest}
        gui={'ok':True,'pid':102,'executable_sha256':digest}
        control=NativeControlOriginal(self.selected);flags=[]
        def query(flag,required=True):
            flags.append(flag)
            if flag=='--background-status-json':return background
            if flag=='--gui-network-diagnostics-json':return gui
            self.fail('Wrong native GUI status command')
        with patch.object(control,'query',side_effect=query),patch.object(control,'owners',return_value={101,102}),patch.object(control,'loaded',return_value=True):
            self.assertEqual(control.before(digest)['gui']['pid'],102)
            self.assertEqual(flags,['--background-status-json','--gui-network-diagnostics-json'])
            gui.pop('pid')
            with self.assertRaisesRegex(guard.ReleaseRejected,'GUI process id'):
                control.before(digest)

    @unittest.skipUnless(sys.platform=='darwin','native atomic directory exchange')
    def test_exact_pre_stop_staged_transaction_can_resume_without_recopy_or_replay(self):
        pins=self.current()
        self.before_hook=lambda: (_ for _ in ()).throw(KeyError('pid'))
        with self.assertRaises(KeyError):self.apply(pins)
        journal=self.selected['state']/'upgrade.json';digest=upgrade.gate.hash_file(journal)
        self.assertEqual(json.loads(journal.read_text())['phase'],'staged')
        self.assertEqual(self.app_bytes(),b'old-image');self.before_hook=None
        with self.assertRaises(guard.ReleaseRejected):self.apply(pins,resume='1'*64)
        wrong=dict(self.pins,commit='1'*40)
        with self.assertRaisesRegex(guard.ReleaseRejected,'exact verified'):
            upgrade.apply(self.archive,self.home,wrong,upgrade.gate.hash_file(self.archive),pins,resume_sha=digest)
        result=self.apply(pins,resume=digest)
        self.assertEqual(result['phase'],'completed');self.assertEqual(result['resumed_from_sha256'],digest)
        self.assertEqual(self.calls,['before','before','stop','start'])
        self.assertEqual(self.app_bytes(),b'new-image')

    @unittest.skipUnless(sys.platform=='darwin','native atomic directory exchange')
    def test_explicit_no_new_rollback_resumes_staged_conflict_and_preserves_old_slot(self):
        root=self.selected['app'].parent/'.desktop-upgrade.rollback'
        root.mkdir(mode=0o700)
        (root/'historical').write_bytes(b'preserve-prior-slot')
        pins=self.current()
        with self.assertRaises((guard.ReleaseRejected, OSError)):
            self.apply(pins)
        self.assertEqual(self.calls,['before'])
        journal=self.selected['state']/'upgrade.json'
        digest=upgrade.gate.hash_file(journal)
        self.assertEqual(json.loads(journal.read_text())['phase'],'staged')
        self.assertEqual((root/'historical').read_bytes(),b'preserve-prior-slot')
        with self.assertRaisesRegex(guard.ReleaseRejected,'incomplete'):
            self.apply(pins,no_new=True)
        result=self.apply(pins,resume=digest,no_new=True)
        self.assertEqual(result['phase'],'completed')
        self.assertEqual(result['rollback_policy'],'none_new')
        self.assertEqual(result['rollback']['new_retained'],False)
        self.assertTrue(result['rollback']['prior_slot_preserved'])
        self.assertEqual((root/'historical').read_bytes(),b'preserve-prior-slot')
        self.assertEqual(self.app_bytes(),b'new-image')
        self.assertEqual(json.loads(self.registry.read_text())['rollback_policy'],'none_new')
        self.assertFalse((self.selected['app'].parent/'.desktop-upgrade.pending').exists())
        self.assertEqual(self.calls,['before','before','stop','start'])

    @unittest.skipUnless(sys.platform=='darwin','native atomic directory exchange')
    def test_no_new_rollback_rejects_prior_slot_change_before_exchange(self):
        root=self.selected['app'].parent/'.desktop-upgrade.rollback'
        root.mkdir(mode=0o700)
        marker=root/'historical';marker.write_bytes(b'prior-slot')
        self.stop_hook=lambda: marker.write_bytes(b'changed')
        with self.assertRaisesRegex(guard.ReleaseRejected,'Prior rollback slot changed'):
            self.apply(no_new=True)
        self.assertEqual(self.app_bytes(),b'old-image')
        self.assertEqual(json.loads(self.registry.read_text()),self.record)
        self.assertNotIn('start',self.calls)

    @unittest.skipUnless(sys.platform=='darwin','native atomic directory exchange')
    def test_no_new_rollback_without_old_slot_creates_none(self):
        result=self.apply(no_new=True)
        self.assertTrue(result['applied'])
        self.assertFalse(result['rollback']['prior_slot_preserved'])
        self.assertFalse((self.selected['app'].parent/'.desktop-upgrade.rollback').exists())
        self.assertEqual(self.app_bytes(),b'new-image')

    @unittest.skipUnless(sys.platform=='darwin','native atomic directory exchange')
    def test_no_new_rollback_activation_failure_keeps_staged_old_app_and_prior_slot(self):
        root=self.selected['app'].parent/'.desktop-upgrade.rollback'
        root.mkdir(mode=0o700)
        (root/'historical').write_bytes(b'prior-slot')
        self.start_hook=lambda: (_ for _ in ()).throw(guard.ReleaseRejected('activation failed'))
        with self.assertRaisesRegex(guard.ReleaseRejected,'activation failed'):
            self.apply(no_new=True)
        self.assertEqual(self.app_bytes(),b'new-image')
        self.assertEqual((root/'historical').read_bytes(),b'prior-slot')
        staged=self.selected['app'].parent/'.desktop-upgrade.pending/Weavelo.app/Contents/MacOS/weavelo'
        self.assertEqual(staged.read_bytes(),b'old-image')
        self.assertEqual(json.loads(self.registry.read_text()),self.record)

    def test_explicit_resume_never_crosses_an_unconfirmed_stop(self):
        pins=self.current()
        self.stop_hook=lambda: (_ for _ in ()).throw(guard.ReleaseRejected('declined'))
        with self.assertRaises(guard.ReleaseRejected):self.apply(pins)
        journal=self.selected['state']/'upgrade.json';digest=upgrade.gate.hash_file(journal)
        self.assertEqual(json.loads(journal.read_text())['phase'],'stopping')
        self.stop_hook=None
        with self.assertRaisesRegex(guard.ReleaseRejected,'pre-stop staged'):
            self.apply(pins,resume=digest)
        self.assertEqual(self.calls,['before','stop']);self.assertEqual(self.app_bytes(),b'old-image')

    @unittest.skipUnless(sys.platform=='darwin','native atomic directory exchange')
    def test_current_update_resumes_after_confirmed_unloaded_stop_without_repeating_stop_or_copy(self):
        pins=self.current();membership=upgrade.membership_metadata(self.selected['profile'])
        plist=self.selected['plist'].read_bytes()
        self.stop_hook=lambda: (_ for _ in ()).throw(guard.ReleaseRejected('declined'))
        with self.assertRaises(guard.ReleaseRejected):self.apply(pins,no_new=True)
        journal=self.selected['state']/'upgrade.json';digest=upgrade.gate.hash_file(journal)
        self.stop_hook=None
        with patch.object(self.control,'before',return_value={'background':None,'gui':None,'loaded':False}), \
             patch.object(shutil,'copytree',side_effect=AssertionError('must reuse the verified stage')):
            result=self.apply(pins,resume=digest,no_new=True)
        self.assertEqual(result['phase'],'completed')
        self.assertTrue(result['resumed_after_verified_stop'])
        self.assertEqual(result['owners_before'],{'background_pid':101,'frontend_pid':102})
        self.assertEqual(self.calls.count('stop'),1)
        self.assertEqual(self.app_bytes(),b'new-image')
        self.assertEqual(upgrade.membership_metadata(self.selected['profile']),membership)
        self.assertEqual(self.selected['plist'].read_bytes(),plist)
        self.assertFalse((self.selected['app'].parent/'.desktop-upgrade.rollback').exists())

    def test_current_stopping_resume_refuses_a_loaded_supervisor_without_changing_the_journal(self):
        pins=self.current()
        self.stop_hook=lambda: (_ for _ in ()).throw(guard.ReleaseRejected('declined'))
        with self.assertRaises(guard.ReleaseRejected):self.apply(pins,no_new=True)
        journal=self.selected['state']/'upgrade.json';digest=upgrade.gate.hash_file(journal)
        with patch.object(self.control,'before',return_value={'background':None,'gui':None,'loaded':True}):
            with self.assertRaisesRegex(guard.ReleaseRejected,'Prior stop must be confirmed'):
                self.apply(pins,resume=digest,no_new=True)
        self.assertEqual(upgrade.gate.hash_file(journal),digest)
        self.assertEqual(self.app_bytes(),b'old-image');self.assertEqual(self.calls.count('stop'),1)

    def test_current_stopping_resume_refuses_a_remaining_owner_without_repeating_stop(self):
        pins=self.current()
        self.stop_hook=lambda: (_ for _ in ()).throw(guard.ReleaseRejected('declined'))
        with self.assertRaises(guard.ReleaseRejected):self.apply(pins,no_new=True)
        journal=self.selected['state']/'upgrade.json';digest=upgrade.gate.hash_file(journal)
        with patch.object(self.control,'before',return_value={'background':{'pid':101},'gui':None,'loaded':False}):
            with self.assertRaisesRegex(guard.ReleaseRejected,'Prior stop must be confirmed'):
                self.apply(pins,resume=digest,no_new=True)
        self.assertEqual(upgrade.gate.hash_file(journal),digest)
        self.assertEqual(self.app_bytes(),b'old-image');self.assertEqual(self.calls.count('stop'),1)

    def test_current_stopping_resume_preserves_a_changed_historical_rollback(self):
        root=self.selected['app'].parent/'.desktop-upgrade.rollback';root.mkdir(mode=0o700)
        (root/'historical').write_bytes(b'prior-slot');pins=self.current()
        self.stop_hook=lambda: (_ for _ in ()).throw(guard.ReleaseRejected('declined'))
        with self.assertRaises(guard.ReleaseRejected):self.apply(pins,no_new=True)
        journal=self.selected['state']/'upgrade.json';digest=upgrade.gate.hash_file(journal)
        (root/'historical').write_bytes(b'changed-by-user')
        with patch.object(self.control,'before',return_value={'background':None,'gui':None,'loaded':False}):
            with self.assertRaisesRegex(guard.ReleaseRejected,'rollback slot changed after stop'):
                self.apply(pins,resume=digest,no_new=True)
        self.assertEqual((root/'historical').read_bytes(),b'changed-by-user')
        self.assertEqual(upgrade.gate.hash_file(journal),digest)
        self.assertEqual(self.app_bytes(),b'old-image');self.assertEqual(self.calls.count('stop'),1)

    def test_changed_staged_candidate_is_preserved_and_never_resumed(self):
        pins=self.current()
        self.before_hook=lambda: (_ for _ in ()).throw(KeyError('pid'))
        with self.assertRaises(KeyError):self.apply(pins)
        journal=self.selected['state']/'upgrade.json';digest=upgrade.gate.hash_file(journal)
        staged=self.selected['app'].parent/'.desktop-upgrade.pending/Weavelo.app/Contents/MacOS/weavelo'
        staged.write_bytes(b'user-changed-staged-image');self.before_hook=None
        with self.assertRaisesRegex(guard.ReleaseRejected,'Staged signed application differs'):
            self.apply(pins,resume=digest)
        self.assertEqual(staged.read_bytes(),b'user-changed-staged-image')
        self.assertEqual(upgrade.gate.hash_file(journal),digest)
        self.assertEqual(self.calls,['before'])


class NativeExitWatchTests(unittest.TestCase):
    @unittest.skipUnless(sys.platform=='darwin','native kernel exit watcher')
    def test_native_kqueue_registers_and_closes_on_actual_child_exit(self):
        child=subprocess.Popen([sys.executable,'-c','import sys;sys.stdin.buffer.read()'],
                               stdin=subprocess.PIPE,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        try:
            with upgrade.process_exit_watch(child.pid) as watch:
                descriptor=watch.fileno()
                child.stdin.close()
                events=watch.control(None,1,10)
                self.assertEqual([event.ident for event in events],[child.pid])
                self.assertEqual(child.wait(timeout=5),0)
            with self.assertRaises(OSError):os.fstat(descriptor)
        finally:
            if child.poll() is None:
                child.stdin.close()
                child.wait(timeout=5)

    @unittest.skipUnless(sys.platform=='darwin','native kernel exit watcher')
    def test_native_kqueue_closes_if_action_raises(self):
        with self.assertRaisesRegex(RuntimeError,'pre-quit failure'):
            with upgrade.process_exit_watch(os.getpid()) as watch:
                descriptor=watch.fileno()
                raise RuntimeError('pre-quit failure')
        with self.assertRaises(OSError):os.fstat(descriptor)



class PersistentRollbackTests(unittest.TestCase):
    setUp = UpgradeTests.setUp
    make_app = UpgradeTests.make_app
    signed = UpgradeTests.signed
    archive_view = UpgradeTests.archive_view
    current = UpgradeTests.current
    apply = UpgradeTests.apply
    app_bytes = UpgradeTests.app_bytes
    @unittest.skipUnless(sys.platform == 'darwin', 'native atomic directory exchange')
    def test_verified_old_slot_and_explicit_restore_preserve_exact_registry_and_profile(self):
        import desktop_upgrade_rollback as rollback
        old_registry = self.registry.read_bytes()
        profile = upgrade.membership_metadata(self.selected['profile'])
        self.apply()
        root = rollback.slot(self.selected)
        self.assertEqual((root/'Weavelo.app/Contents/MacOS/weavelo').read_bytes(), b'old-image')
        self.assertEqual((root/'install.before.json').read_bytes(), old_registry)
        reviewed = rollback.plan(self.home)
        with patch.object(rollback.native, 'restore_runtime', return_value={'executable_sha256':self.signed(root/'Weavelo.app',guard.load_policy(guard.DESKTOP_POLICY_FILE))['executable_sha256']}):
            result = rollback.rollback(self.home, reviewed['plan_sha256'])
        self.assertTrue(result['applied'])
        self.assertEqual(self.app_bytes(), b'old-image')
        self.assertEqual(self.registry.read_bytes(), old_registry)
        self.assertEqual(upgrade.membership_metadata(self.selected['profile']), profile)
        self.assertEqual((root/'Weavelo.app/Contents/MacOS/weavelo').read_bytes(), b'new-image')
        with self.assertRaisesRegex(guard.ReleaseRejected, 'already started'):
            rollback.rollback(self.home, rollback.plan(self.home)['plan_sha256'])

    @unittest.skipUnless(sys.platform == 'darwin', 'native atomic directory exchange')
    def test_post_quit_slot_registry_policy_and_pending_conflicts_refuse_before_exchange(self):
        import desktop_upgrade_rollback as rollback
        for change in ('bundle', 'registry', 'active_registry', 'policy', 'pending'):
            with self.subTest(change=change):
                # Each subtest is its own existing fixture; no previous slot is reused.
                fixture = UpgradeTests('test_readonly_plan_preserves_membership_launch_policy_and_user_content')
                fixture.setUp()
                try:
                    fixture.apply(); root = rollback.slot(fixture.selected)
                    reviewed = rollback.plan(fixture.home)
                    def mutate():
                        if change == 'bundle': (root/'Weavelo.app/Contents/MacOS/weavelo').write_bytes(b'unknown')
                        elif change == 'registry': (root/'install.before.json').write_bytes(b'{}')
                        elif change == 'active_registry': fixture.registry.write_bytes(b'{}')
                        elif change == 'policy': fixture.selected['plist'].write_bytes(b'changed')
                        else: (fixture.selected['profile']/'mesh.secret-recovery').symlink_to('missing')
                    fixture.stop_hook = mutate
                    with self.assertRaises((guard.ReleaseRejected, OSError)):
                        rollback.rollback(fixture.home, reviewed['plan_sha256'])
                    self.assertEqual(fixture.app_bytes(), b'new-image')
                    if change == 'active_registry': self.assertEqual(fixture.registry.read_bytes(), b'{}')
                    else: self.assertEqual(json.loads(fixture.registry.read_text())['source'], fixture.pins)
                finally: fixture.doCleanups()

    @unittest.skipUnless(sys.platform == 'darwin', 'native atomic directory exchange')
    def test_pending_profile_and_unknown_preexisting_slot_never_stop_owner(self):
        import desktop_upgrade_rollback as rollback
        root = rollback.slot(self.selected); root.mkdir(mode=0o700)
        (root/'unknown').write_bytes(b'preserve')
        with self.assertRaises((guard.ReleaseRejected, OSError)): self.apply()
        self.assertNotIn('stop', self.calls)
        self.assertEqual(self.app_bytes(), b'old-image')
        self.assertEqual((root/'unknown').read_bytes(), b'preserve')

if __name__=='__main__':unittest.main()
