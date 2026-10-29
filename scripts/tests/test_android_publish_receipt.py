"""Publication names use the built APK's source, independent of the publisher checkout."""
import hashlib
import contextlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).parents[1]))
import verify_release_apk


class AndroidPublicationReceiptTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='weavelo-apk-publication-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.apk = self.root / 'app-release.apk'
        self.apk.write_bytes(b'own publication metadata fixture')
        self.receipt = {'source_commit_observed': '2' * 40,
                        'source_inputs': {'product_commit': '1' * 40, 'producer_commit': '2' * 40},
                        'sha256': hashlib.sha256(self.apk.read_bytes()).hexdigest()}
        script = (Path(__file__).parents[1] / 'publish_android_nas.sh').read_text()
        start = script.index("<<'SOURCE_COMMIT'\n") + len("<<'SOURCE_COMMIT'\n")
        self.code = script[start:script.index('\nSOURCE_COMMIT\n', start)]

    def inspect(self):
        self.apk.with_suffix('.verification.json').write_text(json.dumps(self.receipt))
        return subprocess.run([sys.executable, '-c', self.code, str(self.apk)], cwd=self.root,
                              capture_output=True, text=True)

    def test_artifact_commit_is_used_even_outside_a_git_checkout(self):
        result = self.inspect()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), '1111111')

    def test_modified_apk_is_rejected_before_remote_publication(self):
        self.apk.write_bytes(b'changed after build')
        result = self.inspect()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('bytes differ', result.stderr)

    def test_missing_or_conflicting_source_receipt_is_rejected(self):
        for commit in (None, 'not-a-commit', '3' * 40):
            with self.subTest(commit=commit):
                self.receipt['source_commit_observed'] = commit
                result = self.inspect()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('missing or inconsistent', result.stderr)

    def test_independent_signature_cli_preserves_the_complete_producer_receipt(self):
        receipt = self.apk.with_suffix('.verification.json')
        original = json.dumps(self.receipt, indent=2).encode()
        receipt.write_bytes(original)
        with patch.object(verify_release_apk, 'verify', return_value={'sha256': self.receipt['sha256']}), contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(verify_release_apk.main([str(self.apk)]), 0)
        self.assertEqual(receipt.read_bytes(), original)

    def test_signature_cli_does_not_fabricate_a_build_receipt(self):
        with patch.object(verify_release_apk, 'verify', return_value={'sha256': self.receipt['sha256']}), contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(verify_release_apk.main([str(self.apk)]), 0)
        self.assertFalse(self.apk.with_suffix('.verification.json').exists())


if __name__ == '__main__':
    unittest.main()
