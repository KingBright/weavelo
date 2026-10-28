import json
import os
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from install_macos_unified import renew_octendra_gui_approval


class OctendraApprovalUpgradeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.profile = self.directory / 'profile'
        self.profile.mkdir(mode=0o700)
        self.records = self.directory / 'agent-state'
        self.records.mkdir(mode=0o700)
        self.path = self.records / 'desktop-launch.json'
        self.locator = self.profile / 'octendra-authority.json'
        self.old, self.new = 'a' * 64, 'b' * 64
        self.authority = {'enabled': True, 'observe': True, 'input': False,
                          'capability_version': 7,
                          'identity': {'owner': 'original-owner', 'device_id': 'original-device'},
                          'agent_sha256': 'c' * 64, 'worker_sha256': 'd' * 64,
                          'weavelo': {'profile': str(self.profile), 'generation': '1-2',
                                      'gui_executable_sha256': self.old,
                                      'target': {'peer_device_id': 'original-peer', 'source': 'MainDisplay'}}}
        self.locator.write_text(json.dumps({'desktop_launch': str(self.path)}))
        self.locator.chmod(0o600)
        self.write_authority()

    def write_authority(self):
        self.path.write_text(json.dumps(self.authority))
        self.path.chmod(0o600)
        return self.path.read_bytes()

    def renew(self):
        return renew_octendra_gui_approval(self.profile, self.old, self.new)

    def test_verified_update_preserves_grant_scope_and_only_renews_gui_identity(self):
        result = self.renew()
        self.assertEqual(result, {'state': 'renewed', 'capability_version': 8})
        expected = dict(self.authority, capability_version=8,
                        weavelo=dict(self.authority['weavelo'], gui_executable_sha256=self.new))
        self.assertEqual(json.loads(self.path.read_bytes()), expected)
        self.assertEqual(self.path.stat().st_mode & 0o777, 0o600)
        self.assertNotIn('original-owner', json.dumps(result))
        self.assertEqual(self.renew(), {'state': 'already_current'})

    def test_existing_recovery_can_rebind_the_original_verified_gui(self):
        self.renew()
        result = renew_octendra_gui_approval(self.profile, self.new, self.old)
        self.assertEqual(result, {'state': 'renewed', 'capability_version': 9})
        self.assertEqual(json.loads(self.path.read_bytes())['weavelo'], self.authority['weavelo'])

    def test_disabled_and_missing_approvals_remain_unconfigured(self):
        self.authority['enabled'] = False
        before = self.write_authority()
        self.assertEqual(self.renew(), {'state': 'disabled_preserved'})
        self.assertEqual(self.path.read_bytes(), before)
        self.locator.unlink()
        self.assertEqual(self.renew(), {'state': 'not_configured'})
        self.assertFalse(self.locator.exists())

    def test_other_profile_unapproved_hash_and_insecure_files_are_never_rewritten(self):
        for value, reason in [('other-profile', 'authority_profile_mismatch'),
                              (None, 'authority_previous_gui_mismatch')]:
            with self.subTest(reason=reason):
                self.authority['weavelo']['profile'] = value if value else str(self.profile)
                self.authority['weavelo']['gui_executable_sha256'] = self.old if value else 'e' * 64
                before = self.write_authority()
                self.assertEqual(self.renew()['reason'], reason)
                self.assertEqual(self.path.read_bytes(), before)
        self.path.chmod(0o644)
        self.assertEqual(self.renew()['state'], 'requires_reapproval')
        self.assertEqual(self.path.read_bytes(), before)

    def test_symlink_locator_and_second_locator_are_not_trusted(self):
        before = self.path.read_bytes()
        actual = self.directory / 'locator-target'
        actual.write_bytes(self.locator.read_bytes())
        actual.chmod(0o600)
        self.locator.unlink()
        self.locator.symlink_to(actual)
        self.assertEqual(self.renew()['state'], 'requires_reapproval')
        self.assertEqual(self.path.read_bytes(), before)
        self.locator.unlink()
        self.locator.write_bytes(actual.read_bytes())
        self.locator.chmod(0o600)
        second = self.profile / 'gui-state'
        second.mkdir(mode=0o700)
        (second / 'octendra-authority.json').write_bytes(actual.read_bytes())
        self.assertEqual(self.renew()['reason'], 'authority_locator_ambiguous')
        self.assertEqual(self.path.read_bytes(), before)


if __name__ == '__main__':
    unittest.main()
