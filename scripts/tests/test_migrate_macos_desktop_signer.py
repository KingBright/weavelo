"""Disposable signer-transition fixtures. No real app/service/permission writes."""
import contextlib
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import plistlib
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import migrate_macos_desktop_signer as migration
import install_macos_desktop as installer
import upgrade_macos_desktop as up
import macos_release_guard as guard


class Fixture:
    def __init__(self):
        self.stack = contextlib.ExitStack()
        self.home = Path(self.stack.enter_context(tempfile.TemporaryDirectory())).resolve() / 'home'
        self.home.mkdir(mode=0o700)
        self.c = migration.contract()
        self.image_builds = {'old': self.c['old_build'], 'new': self.c['new_build']}
        self.c.update(home=str(self.home), uid=os.getuid())
        self.p = up.desktop.paths(self.home)
        for path in (self.p['app'], self.p['profile'], self.p['state'], self.p['plist'].parent):
            path.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.make_app(self.p['app'], b'old')
        self.candidate = self.home / 'candidate/Weavelo.app'; self.make_app(self.candidate, b'new')
        self.retained = self.home / 'Applications/Weavelo.app/sentinel'
        self.retained.parent.mkdir(); self.retained.write_bytes(b'untouched retained product')
        for name in ('mesh.conf', 'mesh.secret'):
            path = self.p['profile'] / name; path.write_bytes(b'synthetic-private-fixture'); path.chmod(0o600)
        self.p['plist'].write_bytes(plistlib.dumps(up.desktop.launch_agent(self.home))); self.p['plist'].chmod(0o600)
        self.registry = self.p['state'] / 'install.json'
        self.original = {'application_id': self.c['bundle_id'], 'applied': True, 'phase': 'upgrade_completed',
                         'source': self.c['old_source'], 'paths': {k: str(v) for k, v in self.p.items()},
                         'user_notes': {'keep': 'original content'}}
        # Deliberately noncanonical formatting; rollback must restore exact bytes.
        self.registry.write_text(json.dumps(self.original, separators=(',', ':'))); self.registry.chmod(0o600)
        self.upgrade = self.p['state'] / 'upgrade.json'
        self.upgrade.write_text(json.dumps({'schema': 1, 'phase': 'completed'})); self.upgrade.chmod(0o600)
        self.archive = self.home / 'candidate.zip'; self.archive.write_bytes(b'synthetic pinned archive')
        self.c['archive_sha256'] = up.gate.hash_file(self.archive)
        self.c['new_executable_sha256'] = self.image_sha(b'new')
        self.c['new_tree_sha256'] = up.tree_digest(self.candidate)
        self.c['current'] = {'executable': self.image_sha(b'old'), 'info': self.image_sha(b'old-info'),
                             'tree': up.tree_digest(self.p['app']), 'install': up.gate.hash_file(self.registry),
                             'upgrade': up.gate.hash_file(self.upgrade), 'plist': up.gate.hash_file(self.p['plist'])}
        self.calls = []; self.running = 'old'; self.gui = True; self.groups = {'control_sha256': '1'*64, 'discovery_sha256': '2'*64}
        self.stop_hook = self.start_hook = None
        owner = self
        class Control:
            def __init__(self, paths): self.paths = paths
            def before(self, expected):
                owner.calls.append('before')
                if owner.running is not None:
                    if expected != owner.image_sha(owner.running.encode()): raise guard.ReleaseRejected('Unexpected runtime owner')
                    return {'loaded': True, 'gui': {'pid': 102} if owner.gui else None,
                            'background': {'pid': 101, 'network_diagnostics': {'network': {'relay_groups': owner.groups.copy()}}}}
                return {'loaded': True, 'gui': None, 'background': None}
            def stop(self, before):
                owner.calls.append('stop')
                if owner.stop_hook: owner.stop_hook()
                owner.running = None
            def start(self, before, expected):
                owner.calls.append('start')
                if owner.start_hook: owner.start_hook()
                actual = (owner.p['app'] / 'Contents/MacOS/weavelo').read_bytes()
                if expected != owner.image_sha(actual): raise guard.ReleaseRejected('Unexpected activated image')
                groups = before['background']['network_diagnostics']['network']['relay_groups']
                if groups != owner.groups: raise guard.ReleaseRejected('Runtime membership changed')
                owner.running = actual.decode(); owner.gui = bool(before['gui'])
                return {'background_pid': 201, 'frontend_pid': 202 if owner.gui else None, 'executable_sha256': expected}
        for obj, name, value in ((migration, 'contract', lambda: copy.deepcopy(self.c)),
                                 (guard, 'verify_app', self.signed), (guard, 'verified_archive', self.archive_view),
                                 (up.fresh, 'verified_product', lambda *a: {}),
                                 (up, 'product_current', lambda *a: {}), (up, 'NativeControl', Control)):
            self.stack.enter_context(patch.object(obj, name, value))

    def close(self): self.stack.close()
    @staticmethod
    def image_sha(raw): return hashlib.sha256(raw).hexdigest()
    @staticmethod
    def make_app(path, raw):
        (path / 'Contents/MacOS').mkdir(parents=True, exist_ok=True)
        (path / 'Contents/MacOS/weavelo').write_bytes(raw)
        (path / 'Contents/Info.plist').write_bytes(raw + b'-info')

    def signed(self, path, policy, **kw):
        image = (path / 'Contents/MacOS/weavelo').read_bytes()
        leaf = migration.OLD_LEAF if image == b'old' else migration.NEW_LEAF
        if image not in (b'old', b'new') or policy.certificate_sha1 != leaf:
            raise guard.ReleaseRejected('Exact certificate/bytes rejected')
        return {'version': '2.0.0-alpha.9', 'build': self.image_builds[image.decode()],
                'bundle_id': policy.bundle_id, 'certificate_sha1': leaf, 'designated_requirement': policy.requirement,
                'executable_sha256': self.image_sha(image), 'info_sha256': self.image_sha(image + b'-info'),
                'signature_verified': True}

    @contextlib.contextmanager
    def archive_view(self, archive, policy, identity_expectations):
        if identity_expectations != self.c['source']: raise guard.ReleaseRejected('Wrong archive source')
        yield self.candidate, dict(self.signed(self.candidate, policy),
                                   archive_sha256=up.gate.hash_file(archive), size_bytes=archive.stat().st_size)

    def plan(self): return migration.plan(self.archive, self.home)
    def apply(self, digest=None): return migration.apply(self.archive, self.home, digest or self.plan()['plan_sha256'])
    def status(self): return migration.status(self.home)
    def rollback(self): return migration.rollback(self.home, self.status()['transaction_sha256'])
    def image(self): return (self.p['app'] / 'Contents/MacOS/weavelo').read_bytes()
    def rollback_app(self): return self.p['app'].parent / '.desktop-signer-rollback/Weavelo.app'


class SignerTransitionTests(unittest.TestCase):
    def setUp(self):
        self.f = Fixture(); self.addCleanup(self.f.close)

    def test_plan_is_readonly_and_pins_exact_dual_requirements(self):
        f = self.f; before = (f.registry.read_bytes(), up.membership_metadata(f.p['profile']), f.p['plist'].read_bytes())
        value = f.plan()
        self.assertIn(migration.OLD_LEAF, value['old_requirement']); self.assertIn(migration.NEW_LEAF, value['new_requirement'])
        self.assertEqual(before, (f.registry.read_bytes(), up.membership_metadata(f.p['profile']), f.p['plist'].read_bytes()))
        self.assertFalse(f.calls); self.assertFalse((f.p['state'] / 'signer-migration.json').exists())
        self.assertFalse(f.rollback_app().parent.exists())

    def test_current_pin_conflicts_and_wrong_owner_refuse_without_stopping(self):
        f = self.f
        for name in f.c['current']:
            with self.subTest(pin=name):
                original = f.c['current'][name]; f.c['current'][name] = '0'*64
                with self.assertRaises(guard.ReleaseRejected): f.plan()
                f.c['current'][name] = original
        f.c['uid'] += 1
        with self.assertRaisesRegex(guard.ReleaseRejected, 'owner'): f.plan()
        self.assertFalse(f.calls)

    def test_candidate_digest_source_certificate_and_build_are_each_pinned(self):
        f = self.f
        for field, wrong in [('archive_sha256', '0'*64), ('new_build', '20261026.1'),
                              ('new_executable_sha256', '0'*64), ('new_tree_sha256', '0'*64)]:
            original = f.c[field]; f.c[field] = wrong
            with self.assertRaises(guard.ReleaseRejected): f.plan()
            f.c[field] = original
        with patch.object(up.fresh, 'verified_product', side_effect=guard.ReleaseRejected('source')):
            with self.assertRaisesRegex(guard.ReleaseRejected, 'source'): f.plan()
        with patch.object(guard, 'load_policy', return_value=guard.Policy(f.c['bundle_id'], 'weavelo', 'Weavelo', '0'*40)):
            with self.assertRaisesRegex(guard.ReleaseRejected, 'policy'): f.plan()
        self.assertFalse(f.calls)

    def test_normal_upgrade_still_rejects_signer_change(self):
        f = self.f
        with self.assertRaisesRegex(guard.ReleaseRejected, 'certificate_sha1'):
            guard.check_upgrade(f.signed(f.p['app'], migration.policies(f.c)[0]),
                                f.signed(f.candidate, migration.policies(f.c)[1]))
        with self.assertRaisesRegex(guard.ReleaseRejected, 'certificate'):
            up.inspect(f.home)

    def test_existing_unknown_backup_stage_or_journal_is_never_overwritten(self):
        f = self.f
        for path in [f.rollback_app().parent, f.p['app'].parent / '.desktop-upgrade.pending',
                     f.p['state'] / 'signer-migration.json']:
            path.mkdir() if path.name != 'signer-migration.json' else path.write_text('unknown')
            with self.assertRaises(guard.ReleaseRejected): f.plan()
            path.rmdir() if path.is_dir() else path.unlink()
        self.assertFalse(f.calls)

    def test_plan_drift_refuses_before_journal(self):
        f = self.f; reviewed = f.plan()['plan_sha256']
        path = f.p['profile'] / 'mesh.conf'; s = path.stat(); os.utime(path, ns=(s.st_atime_ns, s.st_mtime_ns+1000000))
        with self.assertRaisesRegex(guard.ReleaseRejected, 'drift'): f.apply(reviewed)
        self.assertFalse(f.calls)

    def test_unknown_history_is_preserved_without_stopping_or_creating_journal(self):
        f = self.f
        value = dict(f.original, history={'unknown': 'user format'})
        f.registry.write_text(json.dumps(value)); f.c['current']['install'] = up.gate.hash_file(f.registry)
        with self.assertRaisesRegex(guard.ReleaseRejected, 'history'): f.apply()
        self.assertFalse(f.calls); self.assertEqual(json.loads(f.registry.read_text()), value)
        self.assertEqual(f.status()['phase'], 'not_started')

    def test_no_known_background_owner_never_creates_migration_transaction(self):
        f = self.f; f.running = None
        with self.assertRaisesRegex(guard.ReleaseRejected, 'background'): f.apply()
        self.assertEqual(f.calls, ['before']); self.assertEqual(f.status()['phase'], 'not_started')

    def test_hardlinked_registry_and_redirected_rollback_are_refused(self):
        f = self.f; alias = f.registry.with_name('alias'); os.link(f.registry, alias)
        with self.assertRaisesRegex(guard.ReleaseRejected, 'Private'): f.plan()
        alias.unlink()
        f.rollback_app().parent.symlink_to(f.retained.parent, target_is_directory=True)
        with self.assertRaisesRegex(guard.ReleaseRejected, 'Prior transition'): f.plan()
        self.assertFalse(f.calls)

    def test_declined_normal_quit_preserves_old_image_and_blocks_forward_replay(self):
        f = self.f
        f.stop_hook = lambda: (_ for _ in ()).throw(guard.ReleaseRejected('quit declined'))
        reviewed = f.plan()['plan_sha256']
        with self.assertRaisesRegex(guard.ReleaseRejected, 'declined'): f.apply(reviewed)
        self.assertEqual(f.image(), b'old'); self.assertEqual(f.status()['transaction']['phase'], 'stopping')
        with self.assertRaisesRegex(guard.ReleaseRejected, 'Prior transition'): f.apply(reviewed)
        self.assertEqual(f.calls, ['before', 'stop'])

    def test_metadata_or_membership_race_is_preserved_and_blocks_exchange(self):
        f = self.f
        def drift():
            path = f.p['profile'] / 'mesh.conf'; s = path.stat(); os.utime(path, ns=(s.st_atime_ns, s.st_mtime_ns+1000000))
        f.stop_hook = drift
        with self.assertRaisesRegex(guard.ReleaseRejected, 'Membership'): f.apply()
        self.assertEqual(f.image(), b'old'); self.assertNotIn('start', f.calls)

    @unittest.skipUnless(sys.platform == 'darwin', 'native atomic directory exchange')
    def test_success_retains_verified_old_app_and_exact_metadata_without_data_copy(self):
        f = self.f; membership = up.membership_metadata(f.p['profile']); original = f.registry.read_bytes()
        result = f.apply(); root = f.rollback_app().parent
        self.assertTrue(result['applied']); self.assertEqual(result['phase'], 'completed')
        self.assertEqual(f.image(), b'new'); self.assertEqual((f.rollback_app()/'Contents/MacOS/weavelo').read_bytes(), b'old')
        self.assertEqual((root/'install.before.json').read_bytes(), original)
        self.assertEqual(set(v.name for v in root.iterdir()),
                         {'Weavelo.app', 'install.before.json', 'upgrade.before.json', 'launch-agent.before.plist'})
        self.assertEqual(membership, up.membership_metadata(f.p['profile']))
        self.assertEqual(f.retained.read_bytes(), b'untouched retained product')
        self.assertEqual(json.loads(f.registry.read_text())['user_notes'], f.original['user_notes'])
        self.assertFalse(f.status()['forward_resume_supported'])
        self.assertEqual(f.status()['observed']['active']['side'], 'new')
        self.assertEqual(f.status()['observed']['rollback']['side'], 'old')
        self.assertEqual(f.status()['observed']['registry']['side'], 'new')
        with self.assertRaises(guard.ReleaseRejected): f.apply(result['plan_sha256'])

    @unittest.skipUnless(sys.platform == 'darwin', 'native atomic directory exchange')
    def test_completed_rollback_restores_exact_registry_bytes_and_keeps_new_candidate(self):
        f = self.f; old = f.registry.read_bytes(); f.apply()
        # A legitimate later data change is preserved; no profile snapshot is restored.
        (f.p['profile'] / 'mesh.conf').write_bytes(b'updated synthetic user preference')
        membership = up.membership_metadata(f.p['profile'])
        f.groups = {'control_sha256': '3'*64, 'discovery_sha256': '4'*64}
        self.assertTrue(f.rollback()['rolled_back']); self.assertEqual(f.image(), b'old')
        self.assertEqual(f.registry.read_bytes(), old)
        self.assertEqual(membership, up.membership_metadata(f.p['profile']))
        self.assertEqual((f.rollback_app()/'Contents/MacOS/weavelo').read_bytes(), b'new')
        self.assertEqual(f.running, 'old')
        with self.assertRaisesRegex(guard.ReleaseRejected, 'already completed'): f.rollback()

    @unittest.skipUnless(sys.platform == 'darwin', 'native atomic directory exchange')
    def test_activation_failure_retains_both_sides_and_explicit_rollback_restores_old(self):
        f = self.f; original = f.registry.read_bytes()
        f.start_hook = lambda: (_ for _ in ()).throw(guard.ReleaseRejected('activation refused'))
        with self.assertRaisesRegex(guard.ReleaseRejected, 'activation'): f.apply()
        self.assertEqual(f.image(), b'new'); self.assertEqual(f.registry.read_bytes(), original)
        self.assertEqual(f.status()['transaction']['phase'], 'activating')
        f.start_hook = None; self.assertTrue(f.rollback()['rolled_back']); self.assertEqual(f.image(), b'old')

    @unittest.skipUnless(sys.platform == 'darwin', 'native atomic directory exchange')
    def test_tampered_bundle_backup_registry_plist_or_receipt_never_stops_runtime(self):
        f = self.f; f.apply(); root = f.rollback_app().parent
        targets = [f.rollback_app()/'Contents/MacOS/weavelo', root/'install.before.json', f.registry, f.p['plist']]
        for target in targets:
            old = target.read_bytes(); count = len(f.calls); target.write_bytes(old+b'changed')
            with self.assertRaises((guard.ReleaseRejected, ValueError)): f.rollback()
            self.assertEqual(len(f.calls), count); target.write_bytes(old)
        count = len(f.calls)
        with self.assertRaisesRegex(guard.ReleaseRejected, 'version conflict'):
            migration.rollback(f.home, '0'*64)
        self.assertEqual(len(f.calls), count)

    @unittest.skipUnless(sys.platform == 'darwin', 'native atomic directory exchange')
    def test_unknown_rollback_content_is_preserved_without_stopping(self):
        f = self.f; f.apply(); extra = f.rollback_app().parent/'unknown'; extra.write_bytes(b'keep')
        count = len(f.calls)
        with self.assertRaisesRegex(guard.ReleaseRejected, 'Unexpected rollback contents'): f.rollback()
        self.assertEqual(len(f.calls), count); self.assertEqual(extra.read_bytes(), b'keep')

    @unittest.skipUnless(sys.platform == 'darwin', 'native atomic directory exchange')
    def test_interrupted_forward_phases_only_support_explicit_exact_rollback(self):
        boundaries = ['staged', 'stopping', 'stopped', 'exchanging', 'exchanged',
                      'activating', 'activated', 'committing', 'completed']
        for boundary in boundaries:
            with self.subTest(phase=boundary):
                f = Fixture()
                try:
                    original = f.registry.read_bytes(); reviewed = f.plan()['plan_sha256']
                    real = migration.checkpoint
                    def interrupted(p, data, phase, expected):
                        result = real(p, data, phase, expected)
                        if phase == boundary: raise KeyboardInterrupt()
                        return result
                    with patch.object(migration, 'checkpoint', side_effect=interrupted):
                        with self.assertRaises(KeyboardInterrupt): f.apply(reviewed)
                    self.assertEqual(f.status()['transaction']['phase'], boundary)
                    with self.assertRaisesRegex(guard.ReleaseRejected, 'Prior transition'): f.apply(reviewed)
                    self.assertTrue(f.rollback()['rolled_back'])
                    self.assertEqual(f.image(), b'old'); self.assertEqual(f.registry.read_bytes(), original)
                    self.assertEqual(f.running, 'old')
                finally: f.close()

    @unittest.skipUnless(sys.platform == 'darwin', 'native atomic directory exchange')
    def test_interrupted_rollback_phases_reobserve_exact_tuple_without_forward_replay(self):
        for boundary in ['rollback_stopping', 'rollback_stopped', 'rollback_exchanging',
                         'rollback_exchanged', 'rollback_registry', 'rollback_activating']:
            with self.subTest(phase=boundary):
                f = Fixture()
                try:
                    original = f.registry.read_bytes(); f.apply()
                    # Preserve changed live groups even after a rollback interruption.
                    f.groups = {'control_sha256': '5'*64, 'discovery_sha256': '6'*64}
                    real = migration.checkpoint
                    def interrupted(p, data, phase, expected):
                        result = real(p, data, phase, expected)
                        if phase == boundary: raise KeyboardInterrupt()
                        return result
                    with patch.object(migration, 'checkpoint', side_effect=interrupted):
                        with self.assertRaises(KeyboardInterrupt): f.rollback()
                    self.assertEqual(f.status()['transaction']['phase'], boundary)
                    self.assertTrue(f.rollback()['rolled_back']); self.assertEqual(f.image(), b'old')
                    self.assertEqual(f.registry.read_bytes(), original); self.assertEqual(f.running, 'old')
                finally: f.close()

    @unittest.skipUnless(sys.platform == 'darwin', 'native atomic directory exchange')
    def test_exchange_completed_before_error_is_resolved_from_actual_tuple(self):
        f = self.f; real = up.exchange
        def interrupted(source, target):
            real(source, target)
            raise KeyboardInterrupt()
        with patch.object(up, 'exchange', side_effect=interrupted):
            with self.assertRaises(KeyboardInterrupt): f.apply()
        self.assertEqual(f.status()['transaction']['phase'], 'exchanging'); self.assertEqual(f.image(), b'new')
        self.assertTrue(f.rollback()['rolled_back']); self.assertEqual(f.image(), b'old')

    @unittest.skipUnless(sys.platform == 'darwin', 'native atomic directory exchange')
    def test_registry_committed_before_error_is_recognized_and_restored_exactly(self):
        f = self.f; old = f.registry.read_bytes(); real = up.atomic_record
        def interrupted(path, data, expected):
            result = real(path, data, expected)
            if path == f.registry: raise KeyboardInterrupt()
            return result
        with patch.object(up, 'atomic_record', side_effect=interrupted):
            with self.assertRaises(KeyboardInterrupt): f.apply()
        self.assertEqual(f.status()['transaction']['phase'], 'committing')
        self.assertNotEqual(f.registry.read_bytes(), old)
        self.assertTrue(f.rollback()['rolled_back']); self.assertEqual(f.registry.read_bytes(), old)

    def test_partial_pre_stage_state_is_preserved_without_guessing_cleanup(self):
        f = self.f; real = migration.write_new
        def interrupted(path, raw):
            real(path, raw)
            if path.name == 'install.before.json': raise KeyboardInterrupt()
        with patch.object(migration, 'write_new', side_effect=interrupted):
            with self.assertRaises(KeyboardInterrupt): f.apply()
        count = len(f.calls)
        self.assertEqual(f.status()['transaction']['phase'], 'prepared')
        self.assertEqual(f.status()['observed']['active']['side'], 'old')
        self.assertFalse(f.status()['observed']['rollback']['signature_verified'])
        with self.assertRaises(guard.ReleaseRejected): f.rollback()
        self.assertEqual(len(f.calls), count); self.assertEqual(f.image(), b'old')
        self.assertEqual((f.rollback_app().parent/'install.before.json').read_bytes(), f.registry.read_bytes())

    def test_install_dispatch_keeps_modes_separate_and_propagates_failure_exit(self):
        with patch.object(sys, 'argv', ['install_macos_desktop.py', '--migrate-signer', '--status']), \
             patch.object(migration, 'cli', return_value=7) as cli:
            with self.assertRaises(SystemExit) as result: installer.main()
            self.assertEqual(result.exception.code, 7); cli.assert_called_once()
        with patch.object(sys, 'argv', ['install_macos_desktop.py', '--upgrade', '--migrate-signer']):
            with self.assertRaisesRegex(guard.ReleaseRejected, 'Choose'): installer.main()

    @unittest.skipUnless(sys.platform == 'darwin', 'macOS CLI mode validation')
    def test_apply_without_reviewed_digest_refuses_before_plan_or_mutation(self):
        f = self.f
        with patch.object(sys, 'argv', ['migrate_macos_desktop_signer.py', str(f.archive), '--apply']), \
             contextlib.redirect_stderr(io.StringIO()) as error:
            self.assertEqual(migration.cli(), 1)
        self.assertIn('Reviewed plan digest required', error.getvalue())
        self.assertFalse(f.calls); self.assertEqual(f.status()['phase'], 'not_started')


class FixedContractTests(unittest.TestCase):
    def test_checked_in_contract_matches_authorized_leaves_and_current_target_policy(self):
        c = migration.contract(); old, new = migration.policies(c)
        self.assertNotEqual(old.certificate_sha1, new.certificate_sha1)
        self.assertEqual(old.bundle_id, new.bundle_id)
        self.assertEqual(c['current']['executable'], 'c50d0cafc0cfa2055210fbe7565105c5580f15a95e1dc9f22256e2576ea76d9c')
        self.assertEqual(c['archive_sha256'], 'e9858f65390b0c5ed5f33a0a0cd1e75ca7d80153efb083569d423aed76be7165')

    def test_environment_does_not_override_contract_or_signers(self):
        before = migration.contract()
        with patch.dict(os.environ, {'WEAVELO_SIGNING_IDENTITY': 'wrong', 'WEAVELO_SIGNING_POLICY': '/wrong',
                                     'WEAVELO_MIGRATION_CONTRACT': '/wrong'}):
            self.assertEqual(before, migration.contract())


if __name__ == '__main__': unittest.main()
