"""Preload admission across Weavelo helper entrypoints; disposable roots only."""
import contextlib
import fcntl
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
if os.name != 'posix':
    raise unittest.SkipTest('Native lifecycle preload gate uses POSIX flock')
import lifecycle_preload_gate as guard
import release_artifact_lifecycle as release
import check_artifact_lifecycle as observe
import build_identity_gate as identity


class PreloadAdoptionTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='weavelo-preload-fixture-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve() / 'skills'
        self.root.mkdir(mode=0o700)
        self.gate = guard.Gate(self.root)

    def test_disposable_hold_refuses_before_helper_or_build(self):
        # This state exists only under TemporaryDirectory, never the installed skill.
        self.gate.change('begin', 'fixture', 'absent', 'a' * 64)
        with patch.object(guard, 'Gate', return_value=self.gate), \
             patch.object(release, '_public') as public, \
             patch.object(release, '_producer') as producer, \
             patch.object(observe, '_invoke') as invoke, \
             patch.object(identity, '_observed_build') as build:
            with self.assertRaisesRegex(guard.Refusal, 'gate_held'):
                release.public('release-preflight')
            with self.assertRaisesRegex(guard.Refusal, 'gate_held'):
                with release.producer('android-native'):
                    self.fail('held producer entered')
            with self.assertRaisesRegex(guard.Refusal, 'gate_held'):
                observe.invoke(Path('/unused-helper'), [])
            with self.assertRaisesRegex(guard.Refusal, 'gate_held'):
                identity.observed_build(Path('/unused-source'), {})
            public.assert_not_called()
            producer.assert_not_called()
            invoke.assert_not_called()
            build.assert_not_called()

    def test_producer_keeps_admission_until_finally_completes(self):
        events = []

        @contextlib.contextmanager
        def fake_producer(_kind):
            guard.require_admitted()
            try:
                yield ('fixture', {})
            finally:
                guard.require_admitted()
                events.append('cleanup')

        with patch.object(guard, 'Gate', return_value=self.gate), \
             patch.object(release, '_producer', side_effect=fake_producer):
            with release.producer('android-native'):
                directory = os.open(self.root, os.O_RDONLY | os.O_DIRECTORY)
                try:
                    with self.assertRaises(BlockingIOError):
                        fcntl.flock(directory, fcntl.LOCK_EX | fcntl.LOCK_NB)
                finally:
                    os.close(directory)
            self.assertEqual(events, ['cleanup'])
            directory = os.open(self.root, os.O_RDONLY | os.O_DIRECTORY)
            try:
                fcntl.flock(directory, fcntl.LOCK_EX | fcntl.LOCK_NB)
            finally:
                os.close(directory)


if __name__ == '__main__':
    unittest.main()
