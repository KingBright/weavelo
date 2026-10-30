"""Exercise the production Swift predicate with real SDK NSError identities."""
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest


@unittest.skipUnless(sys.platform == 'darwin' and shutil.which('swiftc'), 'native macOS SDK required')
class CaptureStopStateTests(unittest.TestCase):
    def test_only_the_exact_already_stopped_error_releases_the_native_owner(self):
        root = Path(__file__).resolve().parents[2]
        path = root / 'third_party/screencapturekit/swift-bridge/Sources/ScreenCaptureKitBridge/Stream.swift'
        source = path.read_text()
        predicate = re.search(r'^func captureStopAlreadyCompleted\(_ error: Error\) -> Bool \{.*?^\}', source, re.M | re.S)
        self.assertIsNotNone(predicate)
        stop = source.split('public func stopStreamCapture(', 1)[1].split('/// Updates the content filter', 1)[0]
        self.assertRegex(stop, r'if captureStopAlreadyCompleted\(error\)\s*\{\s*callback\(context, true, nil\)\s*return')
        driver = '''
import Foundation
import ScreenCaptureKit
PREDICATE
let stopped = SCStreamError.Code.attemptToStopStreamState.rawValue
precondition(stopped == -3808)
precondition(captureStopAlreadyCompleted(NSError(domain: SCStreamErrorDomain, code: stopped)))
precondition(!captureStopAlreadyCompleted(NSError(domain: "another.framework", code: stopped)))
for code in [-3801, -3805, -3811, -999, 0] {
    precondition(!captureStopAlreadyCompleted(NSError(domain: SCStreamErrorDomain, code: code)))
}
print("native stopped-state identity checks passed")
'''.replace('PREDICATE', predicate.group(0))
        with tempfile.TemporaryDirectory(prefix='weavelo-stop-state-') as directory:
            program = Path(directory) / 'main.swift'
            binary = Path(directory) / 'stop-state'
            program.write_text(driver)
            built = subprocess.run(['swiftc', str(program), '-o', str(binary)], capture_output=True, text=True, timeout=90)
            self.assertEqual(built.returncode, 0, (built.stdout + built.stderr)[-2500:])
            result = subprocess.run([str(binary)], capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, (result.stdout + result.stderr)[-2500:])
            self.assertIn('identity checks passed', result.stdout)


if __name__ == '__main__':
    unittest.main()
