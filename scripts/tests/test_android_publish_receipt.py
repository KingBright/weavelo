"""Publication names use the built APK's source, independent of the publisher checkout."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


class AndroidPublicationReceiptTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='weavelo-apk-publication-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.apk = self.root / 'app-release.apk'
        self.apk.write_bytes(b'own publication metadata fixture')
        self.receipt = {'source_commit_observed': '1' * 40,
                        'source_inputs': {'product_commit': '1' * 40},
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
        for commit in (None, 'not-a-commit', '2' * 40):
            with self.subTest(commit=commit):
                self.receipt['source_commit_observed'] = commit
                result = self.inspect()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('missing or inconsistent', result.stderr)


if __name__ == '__main__':
    unittest.main()
