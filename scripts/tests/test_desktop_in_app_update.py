import io
import json
from pathlib import Path
import sys
import tempfile
from contextlib import contextmanager
import unittest
import urllib.error
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import desktop_in_app_update as updater
import macos_release_guard as guard


class UpdateAdapterTests(unittest.TestCase):
    def item(self):
        commit = 'a' * 40
        return {'file': 'Weavelo-desktop-macos-arm64-test.zip', 'source_commit': commit,
                'sha256': 'b' * 64, 'size_bytes': 123,
                'build_identity_expectations': {'commit': commit, 'identity_sha256': 'c' * 64,
                                                'manifest_sha256': 'd' * 64}}

    def test_selects_only_one_complete_current_family(self):
        item = self.item()
        other = {'file': 'Weavelo-Windows-x64-test.zip'}
        self.assertEqual(updater.metadata(json.dumps({'schema': 1, 'releases': [other, item]})), item)
        with self.assertRaises(guard.ReleaseRejected):
            updater.metadata(json.dumps({'schema': 1, 'releases': [item, item]}))
        item['source_commit'] = 'f' * 40
        with self.assertRaises(guard.ReleaseRejected):
            updater.metadata(json.dumps({'schema': 1, 'releases': [item]}))

    def test_rejects_invalid_names_and_sizes(self):
        for field, value in [('file', 'Weavelo-desktop-macos-arm64-../evil.zip'),
                             ('size_bytes', 0), ('size_bytes', True), ('sha256', 'x' * 64)]:
            item = self.item(); item[field] = value
            with self.subTest(field=field, value=value), self.assertRaises(guard.ReleaseRejected):
                updater.metadata(json.dumps({'schema': 1, 'releases': [item]}))

    def test_download_checks_size_and_rejects_redirects(self):
        class Response(io.BytesIO):
            pass
        class Opener:
            def open(self, url, timeout): return Response(b'abc')
        self.assertEqual(updater.download(Opener(), 'https://example.test/', io.BytesIO(), 3, 3),
                         'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad')
        with self.assertRaises(guard.ReleaseRejected):
            updater.download(Opener(), 'https://example.test/', io.BytesIO(), 2)
        with self.assertRaises(guard.ReleaseRejected):
            updater.download(Opener(), 'https://example.test/', io.BytesIO(), 4, 4)
        with self.assertRaises(guard.ReleaseRejected):
            updater.NoRedirect().redirect_request(None, None, 302, '', {}, 'https://example.test/')

    def test_signer_lookup_falls_back_only_for_404(self):
        signer = 'a' * 40
        item = self.item(); item.update(signer_sha1=signer, build='20261109.10')
        raw = json.dumps({'schema': 1, 'releases': [item]}).encode()
        calls = []
        def specific(opener, url, output, limit, expected_size=None):
            calls.append(url); output.write(raw)
        with patch.object(updater, 'download', side_effect=specific):
            selected, dedicated = updater.fetch_metadata(None, signer)
            self.assertEqual(selected, item); self.assertTrue(dedicated)
            self.assertEqual(calls, [updater.BASE + 'desktop-releases-macos-' + signer + '.json'])
        def missing(opener, url, output, limit, expected_size=None):
            calls.append(url)
            if 'macos-' in url: raise urllib.error.HTTPError(url, 404, 'missing', {}, None)
            output.write(raw)
        calls.clear()
        with patch.object(updater, 'download', side_effect=missing):
            selected, dedicated = updater.fetch_metadata(None, signer)
            self.assertEqual(selected, item); self.assertFalse(dedicated)
            self.assertEqual(len(calls), 2)
        calls.clear()
        def forbidden(opener, url, output, limit, expected_size=None):
            calls.append(url); raise urllib.error.HTTPError(url, 403, 'forbidden', {}, None)
        with patch.object(updater, 'download', side_effect=forbidden):
            with self.assertRaises(urllib.error.HTTPError): updater.fetch_metadata(None, signer)
            self.assertEqual(len(calls), 1)
        item['signer_sha1'] = 'b' * 40
        raw = json.dumps({'schema': 1, 'releases': [item]}).encode()
        with patch.object(updater, 'download', side_effect=specific):
            with self.assertRaises(guard.ReleaseRejected): updater.fetch_metadata(None, signer)

    def test_installer_does_not_start_when_installed_signer_is_unverified(self):
        home = Path.home().resolve()
        fake = home / 'Applications/Weavelo/Weavelo.app/Contents/Resources/desktop-updater/scripts/desktop_in_app_update.py'
        original_resolve = Path.resolve
        def resolve(path, *args, **kwargs):
            if path == fake: return fake
            return original_resolve(path, *args, **kwargs)
        with patch.object(updater.sys, 'platform', 'darwin'), \
             patch.object(updater, '__file__', str(fake)), \
             patch.object(Path, 'resolve', autospec=True, side_effect=resolve), \
             patch.object(guard, 'desktop_policy', side_effect=guard.ReleaseRejected('signer mismatch')), \
             patch.object(updater.urllib.request, 'build_opener') as network, \
             patch.object(updater.upgrade, 'apply') as apply:
            with self.assertRaisesRegex(guard.ReleaseRejected, 'signer mismatch'):
                updater.run()
            network.assert_not_called()
            apply.assert_not_called()

    def test_signer_mismatch_never_reaches_installer(self):
        home = Path.home().resolve()
        fake = home / 'Applications/Weavelo/Weavelo.app/Contents/Resources/desktop-updater/scripts/desktop_in_app_update.py'
        original_resolve = Path.resolve
        def resolve(path, *args, **kwargs):
            return fake if path == fake else original_resolve(path, *args, **kwargs)
        item = self.item()
        item['sha256'] = 'e' * 64
        def download(opener, url, output, limit, expected_size=None):
            if url.endswith('desktop-releases.json'):
                output.write(json.dumps({'schema': 1, 'releases': [item]}).encode())
                return 'unused'
            output.write(b'candidate')
            return item['sha256']
        policy = type('Policy', (), {'certificate_sha1': 'pin'})()
        context = {'signed': {'certificate_sha1': 'pin'}, 'pins': {}}
        with patch.object(updater.sys, 'platform', 'darwin'), \
             patch.object(updater, '__file__', str(fake)), \
             patch.object(Path, 'resolve', autospec=True, side_effect=resolve), \
             patch.object(guard, 'desktop_policy', return_value=policy), \
             patch.object(updater.upgrade, 'inspect', return_value=context), \
             patch.object(updater, 'fetch_metadata', return_value=(item, False)), \
             patch.object(updater, 'download', side_effect=download), \
             patch.object(guard, 'verified_archive', side_effect=guard.ReleaseRejected('different signer')), \
             patch.object(updater.upgrade, 'apply') as apply:
            with self.assertRaisesRegex(guard.ReleaseRejected, 'signing or identity verification.*different signer'):
                updater.run()
            apply.assert_not_called()

    def test_verified_candidate_delegates_observed_pins_to_existing_transaction(self):
        home = Path.home().resolve()
        fake = home / 'Applications/Weavelo/Weavelo.app/Contents/Resources/desktop-updater/scripts/desktop_in_app_update.py'
        original_resolve = Path.resolve
        def resolve(path, *args, **kwargs):
            return fake if path == fake else original_resolve(path, *args, **kwargs)
        item = self.item()
        item['sha256'] = 'e' * 64
        def download(opener, url, output, limit, expected_size=None):
            if url.endswith('desktop-releases.json'):
                output.write(json.dumps({'schema': 1, 'releases': [item]}).encode())
                return 'unused'
            output.write(b'candidate')
            return item['sha256']
        policy = type('Policy', (), {'certificate_sha1': 'pin'})()
        context = {'signed': {'certificate_sha1': 'pin'}, 'pins': {'executable': 'current'}, 'tree': 'old-tree'}
        @contextmanager
        def archive(*args, **kwargs):
            yield Path('/candidate'), {'certificate_sha1': 'pin'}
        with patch.object(updater.sys, 'platform', 'darwin'), \
             patch.object(updater, '__file__', str(fake)), \
             patch.object(Path, 'resolve', autospec=True, side_effect=resolve), \
             patch.object(guard, 'desktop_policy', return_value=policy), \
             patch.object(updater.upgrade, 'inspect', return_value=context), \
             patch.object(updater, 'fetch_metadata', return_value=(item, False)), \
             patch.object(updater, 'download', side_effect=download), \
             patch.object(guard, 'verified_archive', side_effect=archive), \
             patch.object(updater.fresh, 'verified_product'), \
             patch.object(updater.upgrade, 'tree_digest', return_value='new-tree'), \
             patch.object(updater, 'release_decision', return_value='compatible_upgrade'), \
             patch.object(updater.upgrade, 'apply', return_value={'applied': True}) as apply:
            self.assertEqual(updater.run(), {'applied': True})
            args = apply.call_args.args
            self.assertEqual(args[1:], (home, item['build_identity_expectations'], item['sha256'], context['pins']))
            self.assertEqual(apply.call_args.kwargs, {'no_new_rollback': True})

    def test_release_direction_uses_signed_build_and_allows_version_jumps(self):
        def signed(build, executable):
            return {'build': build, 'bundle_id': 'id', 'certificate_sha1': 'pin',
                    'designated_requirement': 'requirement', 'executable_sha256': executable,
                    'info_sha256': executable}
        current = signed('20261109.9', 'current')
        self.assertEqual(updater.release_decision(current, current, 'same', 'same'), 'already_installed')
        for candidate in [signed('20261107.148', 'old'), signed('20261109.9', 'different')]:
            with self.assertRaisesRegex(guard.ReleaseRejected, 'not newer'):
                updater.release_decision(current, candidate, 'old', 'new')
        with self.assertRaisesRegex(guard.ReleaseRejected, 'not newer'):
            updater.release_decision(current, current, 'old-resources', 'different-resources')
        # No N-1 or version-label dependency: a much newer full build is accepted.
        self.assertEqual(updater.release_decision(current, signed('20271201.1', 'new'), 'old', 'new'),
                         'compatible_upgrade')

    def test_packager_embeds_allowlisted_modules_and_policies_before_signing(self):
        policy = guard.load_policy(guard.DESKTOP_POLICY_FILE)
        import desktop_identity
        identity = desktop_identity.identity()
        info = {'desktop_identity': identity, 'desktop_runtime': {
            'control_port': identity['control_port'], 'discovery_port': identity['discovery_port'],
            'implicit_relay': False, 'implicit_p2p': False}}
        with tempfile.TemporaryDirectory() as temp:
            stage = Path(temp); binary = stage / 'weavelo'; binary.write_bytes(b'fixture')
            with patch.object(guard, 'run', return_value=json.dumps(info)):
                app = guard.assemble_app(stage, binary, '2.0.0-alpha.9', '123', policy)
            root = app / 'Contents/Resources/desktop-updater'
            for relative in ('scripts/desktop_in_app_update.py', 'scripts/upgrade_macos_desktop.py',
                             'scripts/desktop_upgrade_rollback.py', 'deploy/desktop/product-identity.json',
                             'deploy/macos/desktop-signing-policy.json', 'deploy/macos/book-desktop-signing-policy.json'):
                self.assertEqual((root / relative).read_bytes(), (guard.ROOT / relative).read_bytes())
            self.assertFalse((root / 'scripts').joinpath('__pycache__').exists())

if __name__ == '__main__':
    unittest.main()
