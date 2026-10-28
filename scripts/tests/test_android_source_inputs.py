"""Commit-bound Android input fixtures; no real build, SDK or signing material."""
from pathlib import Path
import copy
import json
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from scripts import android_source_inputs as inputs


class AndroidSourceInputsTests(unittest.TestCase):
    def test_git_root_case_alias_is_the_same_checkout(self):
        original = inputs.git
        def git(root, *args):
            if args == ('rev-parse', '--show-toplevel'):
                return (str(self.root).upper() + '\n').encode()
            return original(root, *args)
        with patch.object(inputs, 'git', side_effect=git), \
             patch.object(Path, 'samefile', lambda a,b:str(a).casefold()==str(b).casefold()):
            self.assertEqual(inputs.snapshot(self.root)['product_commit'], self.commit_id)

    def test_another_git_root_is_rejected(self):
        original = inputs.git
        other = self.root/'another'; other.mkdir()
        def git(root, *args):
            return (str(other)+'\n').encode() if args == ('rev-parse','--show-toplevel') else original(root,*args)
        with patch.object(inputs, 'git', side_effect=git):
            with self.assertRaisesRegex(ValueError, 'expected repository root'):
                inputs.snapshot(self.root)

    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='weavelo-android-inputs-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.git('init', '-b', 'main')
        for name in (*inputs.PRODUCT_FILES, *inputs.PRODUCER_FILES):
            self.write(name, '# fixture\n')
        for package in ('protocol', 'remote_core', 'remote_client_bridge'):
            self.write(package + '/Cargo.toml', '[package]\nname="' + package + '"\nversion="1.0.0"\n')
            self.write(package + '/src/lib.rs', '// fixture\n')
        self.write('.cargo/config.toml', '# fixture\n')
        self.write('android/app/src/main/java/Native.kt', '// fixture\n')
        self.write('app/src/background_service.rs', '// desktop\n')
        self.write('host/src/service.rs', '// host\n')
        self.write('docs/status.md', 'status\n')
        self.write('.gitignore', 'ignored.kt\nandroid/local.properties\n' +
                   '\n'.join('/' + path + '/' for path in inputs.GENERATED) + '\n')
        self.commit()
        self.commit_id = self.git('rev-parse', 'HEAD').strip()

    def git(self, *args):
        return subprocess.check_output(['git', '-c', 'core.hooksPath=/dev/null', '-C', str(self.root), *args],
                                       text=True, stderr=subprocess.DEVNULL)

    def write(self, name, content):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)

    def commit(self):
        self.git('add', '.')
        self.git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                 'commit', '-m', 'fixture')

    def test_only_unrelated_desktop_wip_is_excluded(self):
        before = inputs.snapshot(self.root)
        for name in ('app/src/background_service.rs', 'host/src/service.rs', 'docs/status.md',
                     'app/tests/background_readonly_cli.rs'):
            self.write(name, 'protected WIP\n')
        self.assertTrue(self.git('status', '--porcelain'))
        self.assertEqual(inputs.verify(self.root, before), before)
        self.assertFalse(any(record['path'].startswith(('app/', 'host/src/', 'docs/'))
                             for record in before['product_files']))

    def test_changed_actual_inputs_fail_even_when_git_would_normalize_bytes(self):
        for name in ('remote_client_bridge/src/lib.rs', 'remote_core/src/lib.rs',
                     'protocol/src/lib.rs', 'android/app/src/main/java/Native.kt',
                     'deploy/desktop/product-identity.json', 'Cargo.lock', '.cargo/config.toml',
                     'scripts/build_android_native.sh'):
            with self.subTest(name=name):
                path = self.root / name
                original = path.read_bytes()
                path.write_bytes(original.replace(b'\n', b'\r\n'))
                with self.assertRaisesRegex(ValueError, 'working bytes differ'):
                    inputs.snapshot(self.root)
                path.write_bytes(original)

    def test_untracked_ignored_android_code_is_still_rejected(self):
        self.write('android/app/src/main/java/ignored.kt', 'unexpected code\n')
        self.assertFalse(self.git('status', '--porcelain'))
        with self.assertRaisesRegex(ValueError, 'uncommitted input files'):
            inputs.snapshot(self.root)

    def test_known_generated_outputs_are_verified_by_apk_gate_not_source_inventory(self):
        before = inputs.snapshot(self.root)
        for directory in inputs.GENERATED:
            self.write(directory + '/generated.bin', 'generated\n')
        self.assertEqual(inputs.verify(self.root, before), before)

    def test_missing_or_linked_source_is_rejected(self):
        path = self.root / 'remote_core/src/lib.rs'
        path.unlink()
        with self.assertRaises((OSError, ValueError)):
            inputs.snapshot(self.root)
        path.symlink_to(self.root / 'protocol/src/lib.rs')
        with self.assertRaisesRegex(ValueError, 'linked input'):
            inputs.snapshot(self.root)

    def test_local_sdk_configuration_is_hash_bound_without_recording_its_value(self):
        self.write(inputs.LOCAL_CONFIGURATION, 'sdk.dir=/fixture/sdk\n')
        before = inputs.snapshot(self.root)
        self.assertNotIn('/fixture/sdk', json.dumps(before))
        self.write(inputs.LOCAL_CONFIGURATION, 'sdk.dir=/changed/sdk\n')
        with self.assertRaisesRegex(ValueError, 'inputs changed'):
            inputs.verify(self.root, before)

    def test_product_pin_survives_committed_producer_only_fix(self):
        self.write('scripts/build_android_release.sh', '# reviewed producer fix\n')
        self.commit()
        result = inputs.snapshot(self.root, self.commit_id)
        self.assertEqual(result['product_commit'], self.commit_id)
        self.assertNotEqual(result['producer_commit'], self.commit_id)

    def test_product_pin_cannot_hide_changed_product_or_new_input(self):
        self.write('protocol/src/lib.rs', '// different product\n')
        self.commit()
        with self.assertRaisesRegex(ValueError, 'working bytes differ'):
            inputs.snapshot(self.root, self.commit_id)
        self.write('protocol/src/new.rs', '// new input\n')
        self.commit()
        with self.assertRaisesRegex(ValueError, 'input inventory changed'):
            inputs.snapshot(self.root, self.commit_id)

    def test_new_local_dependency_outside_declared_closure_requires_review(self):
        path = self.root / 'remote_core/Cargo.toml'
        path.write_text(path.read_text() + '\n[dependencies]\nhost={path="../host"}\n')
        self.commit()
        with self.assertRaisesRegex(ValueError, 'dependency requires inventory review'):
            inputs.snapshot(self.root)

    def test_post_build_head_change_and_manifest_tamper_are_rejected(self):
        before = inputs.snapshot(self.root)
        changed = copy.deepcopy(before)
        changed['product_files'][0]['sha256'] = '0' * 64
        with self.assertRaisesRegex(ValueError, 'manifest digest differs'):
            inputs.verify(self.root, changed)
        self.write('docs/status.md', 'committed after producer started\n')
        self.commit()
        with self.assertRaisesRegex(ValueError, 'inputs changed'):
            inputs.verify(self.root, before)


if __name__ == '__main__':
    unittest.main()
