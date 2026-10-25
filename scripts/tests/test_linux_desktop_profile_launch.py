"""Public synthetic startup correction; no live service or environment reads."""
import copy
import json
from pathlib import Path
import unittest
from unittest.mock import patch

import test_upgrade_linux_desktop as upgrade_fixture
import linux_desktop_profile_launch as launch
import upgrade_linux_desktop as up

class ProfileLaunchTests(unittest.TestCase):
    def setUp(self):
        upgrade_fixture.UpgradeTests.setUp(self)
        self.loaded=False;self.reloads=0;self.is_stopped=True
        self.file=launch.location(self.paths);self.file.parent.mkdir(mode=0o700)
        self.environment=self.file.parent/'relay.conf';self.environment.write_bytes(b'synthetic-private-environment-not-for-output');self.environment.chmod(0o600)
        self.original_environment=launch.opaque_pin(self.environment)
        self.control.stopped_state=lambda:self.is_stopped
        def configuration(**kwargs):
            policy={'execution':{'executable':str(self.paths['app']/'weavelo'),'arguments':['--background-service'],'ignore_failure':False}}
            dropins={str(self.environment):dict(launch.opaque_pin(self.environment),opaque_environment_preserved=True)}
            if self.loaded:
                self.assertTrue(launch.authorize(self.paths,preparing=kwargs.get('profile_launch_preparing',False)))
                policy['execution']['canonical_profile_launcher']=launch.arguments(self.paths)
            if self.file.exists():dropins[str(self.file)]=launch.opaque_pin(self.file)
            return {'enabled':'enabled','dropins':dropins,'policy':policy,'active':'inactive' if self.is_stopped else 'active',
                    'pid':0 if self.is_stopped else 101,'canonical_profile_launch':self.loaded}
        self.control.configuration=configuration
        self.c=launch.Correction(self.home,self.control)
        def reload():self.reloads+=1;self.loaded=self.file.exists()
        self.c.reload=reload
    def plan(self):
        p=self.c.plan();return p,up.gate.digest(up.gate.canonical(p))
    def apply(self):
        p,d=self.plan();return self.c.apply(p,d)
    def intact(self):
        self.assertEqual(launch.opaque_pin(self.environment),self.original_environment)
        self.assertEqual(up.profile_pin(self.paths['profile']),self.before_profile)
        self.assertEqual((self.paths['work']/'state.json').read_bytes(),self.original)
    def test_exact_wrapper_and_explicit_stopped_removal_preserve_originals(self):
        r=self.apply();self.assertEqual(r['phase'],'completed');self.assertTrue(launch.authorize(self.paths))
        args=launch.arguments(self.paths)
        doc={'type':'a(sasbttttuii)','data':[[args[0],args,False,*([0]*7)]]}
        actual=up.loaded_exec(doc,self.paths['app']/'weavelo',self.paths)
        self.assertEqual(actual['canonical_profile_launcher'],args)
        for i,value in [(0,'/bin/env'),(1,'OTHER=unknown'),(2,'/unknown/application'),(3,'--headless')]:
            bad=copy.deepcopy(doc);bad['data'][0][1][i]=value
            with self.assertRaises(ValueError):up.loaded_exec(bad,self.paths['app']/'weavelo',self.paths)
        r=self.c.remove(r['record_sha256']);self.assertEqual(r['phase'],'removed');self.assertFalse(self.file.exists());self.intact()
    def test_opaque_environment_is_never_read_or_hashed(self):
        original_read=Path.read_bytes;original_text=Path.read_text
        def read(p,*a,**k):
            self.assertNotEqual(p,self.environment);return original_read(p,*a,**k)
        def text(p,*a,**k):
            self.assertNotEqual(p,self.environment);return original_text(p,*a,**k)
        with patch.object(Path,'read_bytes',read),patch.object(Path,'read_text',text):r=self.apply()
        self.assertNotIn('synthetic-private-environment',self.c.record.read_text());self.intact()
    def test_existing_unknown_file_and_operation_are_not_adopted(self):
        self.file.write_bytes(b'unknown');self.file.chmod(0o600)
        with self.assertRaises(ValueError):self.plan()
        self.assertEqual(self.file.read_bytes(),b'unknown');self.intact()
    def test_forward_apply_cannot_replay(self):
        p,d=self.plan();self.c.apply(p,d);count=self.reloads
        with self.assertRaises(ValueError):self.c.apply(p,d)
        self.assertEqual(self.reloads,count);self.intact()
    def test_reload_timeout_keeps_intent_and_allows_only_explicit_backward_restore(self):
        p,d=self.plan();original=self.c.reload
        def unknown():original();raise TimeoutError('synthetic unknown reload')
        self.c.reload=unknown
        with self.assertRaises(TimeoutError):self.c.apply(p,d)
        r=self.c.status();self.assertEqual(r['phase'],'reload-intent')
        with self.assertRaises(ValueError):self.c.apply(p,d)
        self.c.reload=original;r=self.c.remove(r['record_sha256']);self.assertEqual(r['phase'],'removed');self.intact()
    def test_complete_unrecorded_file_after_write_ack_loss_is_retained(self):
        original=launch.fresh.write_new
        def unknown(*a):original(*a);raise OSError('synthetic write acknowledgement loss')
        p,d=self.plan()
        with patch.object(launch.fresh,'write_new',unknown),self.assertRaises(OSError):self.c.apply(p,d)
        r=self.c.status();self.assertEqual(r['phase'],'create-intent');self.assertTrue(r['matching_owned_file'])
        with self.assertRaises(ValueError):self.c.remove(r['record_sha256'])
        self.assertTrue(self.file.exists());self.intact()
    def test_partial_unknown_file_is_retained(self):
        def partial(path,*_):path.write_bytes(b'partial');path.chmod(0o600);raise OSError('synthetic partial write')
        p,d=self.plan()
        with patch.object(launch.fresh,'write_new',partial),self.assertRaises(OSError):self.c.apply(p,d)
        r=self.c.status()
        with self.assertRaises(ValueError):self.c.remove(r['record_sha256'])
        self.assertEqual(self.file.read_bytes(),b'partial');self.intact()
    def test_live_owner_or_existing_app_transaction_blocks_removal(self):
        r=self.apply();self.is_stopped=False
        with self.assertRaises(ValueError):self.c.remove(r['record_sha256'])
        self.is_stopped=True;self.up.transaction.mkdir(mode=0o700)
        with self.assertRaises(ValueError):self.c.remove(r['record_sha256'])
        self.assertTrue(self.file.exists());self.intact()
    def test_changed_environment_or_starter_identity_refuses(self):
        r=self.apply();self.environment.write_bytes(b'changed-synthetic')
        with self.assertRaises(ValueError):self.c.remove(r['record_sha256'])
        self.assertTrue(self.file.exists())
    def test_wrong_plan_and_unknown_wrapper_refuse_before_changes(self):
        p,d=self.plan()
        with self.assertRaises(ValueError):self.c.apply(p,'0'*64)
        self.assertFalse(self.file.exists());self.assertFalse(self.c.record.exists())
        exe=self.paths['app']/'weavelo'
        doc={'type':'a(sasbttttuii)','data':[['/usr/bin/env',launch.arguments(self.paths),False,*([0]*7)]]}
        with self.assertRaises(ValueError):up.loaded_exec(doc,exe,self.paths)
    def test_systemd_public_path_escaping_is_fixed(self):
        self.assertEqual(launch.quote('a$u%h"\\b'),'"a$$u%%h\\"\\\\b"')
        for value in ('bad\npath','bad\0path'):
            with self.assertRaises(ValueError):launch.quote(value)
    def interrupted_record(self,phase,partial=False):
        original=self.c.save
        def save(record,expected):
            if record['phase']==phase:
                p=self.c.record.with_name(self.c.record.name+'.upgrade-tmp')
                p.write_bytes(b'{' if partial else (json.dumps(record,indent=2)+'\n').encode());p.chmod(0o600)
                raise OSError('synthetic interruption before metadata replace')
            return original(record,expected)
        self.c.save=save;return original
    def test_complete_reload_pending_recovers_backward_only(self):
        original=self.interrupted_record('reload-intent');p,d=self.plan()
        with self.assertRaises(OSError):self.c.apply(p,d)
        r=self.c.status();self.assertTrue(r['pending_metadata']);self.c.save=original
        self.assertEqual(self.c.remove(r['record_sha256'])['phase'],'removed');self.intact()
    def test_complete_completed_pending_recovers_backward_without_forward_commit(self):
        original=self.interrupted_record('completed');p,d=self.plan()
        with self.assertRaises(OSError):self.c.apply(p,d)
        r=self.c.status();self.assertEqual(r['phase'],'reload-intent');self.c.save=original
        self.assertEqual(self.c.remove(r['record_sha256'])['phase'],'removed');self.intact()
    def test_partial_pending_is_preserved(self):
        self.interrupted_record('completed',partial=True);p,d=self.plan()
        with self.assertRaises(OSError):self.c.apply(p,d)
        r=self.c.status()
        with self.assertRaises(ValueError):self.c.remove(r['record_sha256'])
        self.assertTrue(self.file.exists());self.assertEqual(self.c.record.with_name(self.c.record.name+'.upgrade-tmp').read_bytes(),b'{');self.intact()
    def test_failure_before_creating_file_can_restore_backward(self):
        p,d=self.plan()
        with patch.object(launch.fresh,'write_new',side_effect=OSError('synthetic create failure')),self.assertRaises(OSError):self.c.apply(p,d)
        r=self.c.status();self.assertFalse(r['file_exists'])
        self.assertEqual(self.c.remove(r['record_sha256'])['phase'],'removed');self.intact()
    def test_matching_content_with_replaced_inode_does_not_authorize(self):
        r=self.apply();raw=self.file.read_bytes();replacement=self.file.with_name('owned-synthetic-replacement')
        replacement.write_bytes(raw);replacement.chmod(0o600);replacement.replace(self.file)
        with self.assertRaises(ValueError):launch.authorize(self.paths)
        with self.assertRaises(ValueError):self.c.remove(r['record_sha256'])
        self.assertTrue(self.file.exists())
    def test_terminal_removed_pending_completes_metadata_without_reload(self):
        r=self.apply();original=self.interrupted_record('removed')
        with self.assertRaises(OSError):self.c.remove(r['record_sha256'])
        r=self.c.status();self.assertEqual(r['phase'],'remove-intent');self.assertFalse(self.file.exists())
        self.c.save=original;count=self.reloads
        r=self.c.remove(r['record_sha256']);self.assertEqual(r['phase'],'removed');self.assertEqual(self.reloads,count)
        self.c.remove(r['record_sha256']);self.assertEqual(self.reloads,count);self.intact()
    def test_incomplete_or_pending_launch_is_not_upgrade_authorization(self):
        original=self.c.reload
        def unknown():original();raise TimeoutError('synthetic reload unknown')
        self.c.reload=unknown;p,d=self.plan()
        with self.assertRaises(TimeoutError):self.c.apply(p,d)
        with self.assertRaises(ValueError):launch.authorize(self.paths)

if __name__=='__main__':unittest.main()
