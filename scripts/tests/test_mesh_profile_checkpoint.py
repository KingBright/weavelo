"""Synthetic private-backend snapshots only; never read a real user profile."""
import contextlib
import fcntl
import io
import json
import os
from pathlib import Path
import stat
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import mesh_profile_checkpoint as cp


def metadata(network='before', node='local-node', name='原名称'):
    return 'version=weavelo-device-group-v2\n'+''.join(k+'_hex='+v.encode().hex()+'\n' for k,v in
        [('network_name',network),('node_id',node),('display_name',name)])


@unittest.skipUnless(os.name == 'posix' and os.getuid() != 0, 'ordinary-user Unix backend')
class CheckpointTests(unittest.TestCase):
    def setUp(self):
        if sys.platform == 'darwin':
            # Wire/storage-fixture model only; no claim of a macOS ACL adapter.
            acl = patch.object(cp, 'no_extra_acl', return_value=None); acl.start(); self.addCleanup(acl.stop)
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve(); self.root.chmod(0o700)
        (self.root/'gui-state').mkdir(mode=0o700)
        for name, value in [('mesh.conf',metadata()),('mesh.secret','synthetic-before'),
                            ('desktop-instance.lock',''),('gui-state/desktop-instance.lock','')]:
            p=self.root/name;p.write_text(value);p.chmod(0o600)
        self.expected=cp.public(metadata())

    def test_copy_protection_and_no_secret_or_offline_verifier_in_reports(self):
        before=(cp.read(self.root/'mesh.conf'),cp.read(self.root/'mesh.secret'))
        result=cp.create(self.root,self.expected)
        report=json.dumps([result,cp.inspect(self.root)])
        self.assertNotIn('synthetic-before',report)
        self.assertNotIn(cp.hashlib.sha256(b'synthetic-before').hexdigest(),report)
        self.assertEqual(before,(cp.read(self.root/'mesh.conf'),cp.read(self.root/'mesh.secret')))
        p=self.root/cp.CHECKPOINT;m=p.lstat()
        self.assertEqual(stat.S_IMODE(m.st_mode),0o600);self.assertEqual(m.st_uid,os.getuid());self.assertEqual(m.st_nlink,1)
        with self.assertRaises((cp.Refused,OSError)):cp.create(self.root,self.expected)
        self.assertEqual(cp.inspect(self.root)['checkpoint_id'],result['checkpoint_id'])

    def test_known_restore_stages_core_v1_backward_record_without_canonical_writes(self):
        result=cp.create(self.root,self.expected)
        (self.root/'mesh.conf').write_text(metadata('after'))
        (self.root/'mesh.secret').write_text('synthetic-after')
        current=(cp.read(self.root/'mesh.conf'),cp.read(self.root/'mesh.secret'))
        staged=cp.stage_restore(self.root,result['checkpoint_id'],cp.public(metadata('after')))
        self.assertTrue(staged['core_startup_required'])
        self.assertEqual(current,(cp.read(self.root/'mesh.conf'),cp.read(self.root/'mesh.secret')))
        record=json.loads(cp.read(self.root/cp.PENDING))
        self.assertEqual(set(record),{'version','profile','before_metadata','before_secret','after_metadata','after_secret'})
        self.assertEqual(record['before_secret'],'synthetic-before');self.assertEqual(record['after_secret'],'synthetic-after')
        self.assertNotIn('synthetic-',json.dumps(staged))
        with self.assertRaises(cp.Refused):cp.stage_restore(self.root,result['checkpoint_id'],cp.public(metadata('after')))

    def test_live_owner_and_foreign_binding_or_name_refuse(self):
        with (self.root/'desktop-instance.lock').open() as lock:
            fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
            with self.assertRaises((cp.Refused,OSError)):cp.create(self.root,self.expected)
        result=cp.create(self.root,self.expected)
        (self.root/'mesh.conf').write_text(metadata('after',name='用户已改名'))
        with self.assertRaises(cp.Refused):cp.stage_restore(self.root,result['checkpoint_id'],cp.public(metadata('after',name='用户已改名')))
        self.assertFalse((self.root/cp.PENDING).exists())
        with self.assertRaises(cp.Refused):cp.stage_restore(self.root,'wrong-id',cp.public(metadata('after')))

    def test_each_copy_boundary_retains_original_pair_and_protects_unknown_leftovers(self):
        for phase in ['created','written','synced','published','directory-synced']:
            with self.subTest(phase=phase), tempfile.TemporaryDirectory() as directory:
                root=Path(directory).resolve();root.chmod(0o700)
                (root/'gui-state').mkdir(mode=0o700)
                for name,value in [('mesh.conf',metadata()),('mesh.secret','synthetic-before'),('desktop-instance.lock',''),('gui-state/desktop-instance.lock','')]:
                    p=root/name;p.write_text(value);p.chmod(0o600)
                def interrupt(name):
                    if name==phase:raise SystemExit('synthetic interruption')
                with patch.object(cp,'_boundary',side_effect=interrupt),self.assertRaises(SystemExit):cp.create(root,self.expected)
                self.assertEqual(cp.read(root/'mesh.conf'),metadata());self.assertEqual(cp.read(root/'mesh.secret'),'synthetic-before')
                for p in root.iterdir():
                    if p.is_file():self.assertEqual(stat.S_IMODE(p.stat().st_mode),0o600)
                state=cp.inspect(root)
                self.assertTrue(state['checkpoint_present'] or state['unknown_checkpoint_pending'])
                with self.assertRaises((cp.Refused,OSError)):cp.create(root,self.expected)

    def test_recovery_pending_unsafe_checkpoint_and_oversize_refuse_before_source_changes(self):
        (self.root/cp.PENDING).symlink_to('missing')
        with self.assertRaises(cp.Refused):cp.create(self.root,self.expected)
        (self.root/cp.PENDING).unlink()
        old=cp.read(self.root/'mesh.secret')
        with patch.object(cp,'LIMIT',16),self.assertRaises(cp.Refused):cp.create(self.root,self.expected)
        self.assertFalse((self.root/cp.CHECKPOINT).exists());self.assertEqual(cp.read(self.root/'mesh.secret'),old)
        cp.create(self.root,self.expected);(self.root/cp.CHECKPOINT).chmod(0o644)
        with self.assertRaises(cp.Refused):cp.inspect(self.root)
        self.assertEqual(stat.S_IMODE((self.root/cp.CHECKPOINT).stat().st_mode),0o644)

    def test_cli_errors_do_not_echo_private_json_or_change_permissions(self):
        (self.root/cp.CHECKPOINT).write_text('{synthetic-private-body');(self.root/cp.CHECKPOINT).chmod(0o600)
        with self.assertRaises(ValueError):cp.inspect(self.root)
        self.assertEqual(cp.read(self.root/'mesh.secret'),'synthetic-before')

    def test_rust_secret_whitespace_and_strict_metadata_rules(self):
        for marker in ['\x1c','\x1d','\x1e','\x1f']:
            secret=marker+'synthetic'+marker
            (self.root/'mesh.secret').write_text(secret)
            self.assertEqual(cp.pair(self.root)['secret'],secret)
        for invalid in [metadata()+'not-a-field\n',metadata().replace('6265666f7265','62 65 66 6f 72 65'),
                        'network_name_hex=xx\n'+metadata(),metadata().replace('local-node'.encode().hex(),'c0af')]:
            with self.assertRaises((cp.Refused,ValueError,KeyError)):cp.fields(invalid)

    def test_every_restore_boundary_is_visible_without_canonical_replay(self):
        for phase in ['created','written','synced','published','directory-synced']:
            with self.subTest(phase=phase):
                fixture=CheckpointTests();fixture.setUp()
                try:
                    root=fixture.root;r=cp.create(root,fixture.expected)
                    (root/'mesh.conf').write_text(metadata('after'));(root/'mesh.secret').write_text('synthetic-after')
                    def interrupt(name):
                        if name==phase:raise SystemExit('synthetic interruption')
                    with patch.object(cp,'_boundary',side_effect=interrupt),self.assertRaises(SystemExit):
                        cp.stage_restore(root,r['checkpoint_id'],cp.public(metadata('after')))
                    self.assertEqual(cp.read(root/'mesh.secret'),'synthetic-after')
                    state=cp.inspect(root)
                    self.assertTrue(state['core_recovery_pending'] or state['unknown_restore_pending'])
                    with self.assertRaises((cp.Refused,OSError)):cp.stage_restore(root,r['checkpoint_id'],cp.public(metadata('after')))
                finally:fixture.doCleanups()

    def test_extra_acl_and_label_mismatch_fail_before_copying(self):
        with patch.object(cp,'no_extra_acl',side_effect=cp.Refused('synthetic ACL')):
            with self.assertRaises(cp.Refused):cp.create(self.root,self.expected)
        with patch.object(cp,'label',side_effect=lambda path: b'new-label' if '.pending' in path.name else b'old-label'):
            with self.assertRaises(cp.Refused):cp.create(self.root,self.expected)
        pending=self.root/('.'+cp.CHECKPOINT+'.pending')
        self.assertEqual(pending.read_bytes(),b'')
        self.assertEqual(cp.read(self.root/'mesh.secret'),'synthetic-before')

    def test_published_or_existing_checkpoint_label_drift_is_never_accepted(self):
        changed=[False]
        def boundary(phase):
            if phase=='published':changed[0]=True
        def labels(path):
            return b'drifted' if changed[0] and path.name==cp.CHECKPOINT else b'original'
        with patch.object(cp,'_boundary',side_effect=boundary),patch.object(cp,'label',side_effect=labels):
            with self.assertRaises(cp.Refused):cp.create(self.root,self.expected)
            self.assertTrue((self.root/cp.CHECKPOINT).exists())
            with self.assertRaises(cp.Refused):cp.inspect(self.root)
        self.assertEqual(cp.read(self.root/'mesh.secret'),'synthetic-before')

    def test_unrestorable_large_legacy_metadata_is_refused_before_copy(self):
        original=metadata()+'ignored='+'x'*(cp.LIMIT//2)+'\n'
        (self.root/'mesh.conf').write_text(original)
        self.assertEqual(cp.public(original),self.expected)
        with self.assertRaises(cp.Refused):cp.create(self.root,self.expected)
        self.assertFalse((self.root/cp.CHECKPOINT).exists())
        self.assertFalse((self.root/('.'+cp.CHECKPOINT+'.pending')).exists())
        self.assertEqual(cp.read(self.root/'mesh.secret'),'synthetic-before')

    @unittest.skipUnless(sys.platform.startswith('linux'), 'native Linux CLI provider')
    def test_native_cli_never_echoes_malformed_private_record(self):
        import subprocess
        home=self.root/'fake-home';profile=home/'.config/weavelo/desktop';profile.mkdir(parents=True,mode=0o700)
        for path in (home,home/'.config',home/'.config/weavelo'):path.chmod(0o700)
        for name,value in [('mesh.secret','synthetic-before'),(cp.CHECKPOINT,'{synthetic-private-body')]:
            p=profile/name;p.write_text(value);p.chmod(0o600)
        env=dict(os.environ,HOME=str(home))
        result=subprocess.run([sys.executable,'-B',cp.__file__,'inspect'],env=env,capture_output=True,text=True)
        self.assertEqual(result.returncode,1)
        self.assertFalse(json.loads(result.stdout)['ok'])
        self.assertNotIn('synthetic-private-body',result.stdout+result.stderr)
        self.assertNotIn('synthetic-before',result.stdout+result.stderr)
