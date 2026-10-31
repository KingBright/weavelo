"""Normal project delivery does not depend on an external skill maintenance lock."""
import contextlib
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

if os.name != "posix":raise unittest.SkipTest("Project build isolation fixtures use POSIX directory locks")
import fcntl

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import release_artifact_lifecycle as release
import check_artifact_lifecycle as observe
import build_identity_gate as identity


class BuildIsolationTests(unittest.TestCase):
    def test_external_skills_directory_lock_does_not_block_project_entrypoints(self):
        with tempfile.TemporaryDirectory() as directory:
            home=Path(directory);skills=home/'.codex/skills';skills.mkdir(parents=True)
            fd=os.open(skills,os.O_RDONLY|os.O_DIRECTORY)
            try:
                fcntl.flock(fd,fcntl.LOCK_EX|fcntl.LOCK_NB)
                @contextlib.contextmanager
                def producer(_kind):yield ('fixture',{})
                with patch.object(Path,'home',return_value=home), \
                     patch.object(release,'_public',return_value='published') as public, \
                     patch.object(release,'_producer',side_effect=producer), \
                     patch.object(observe,'_invoke',return_value='observed') as invoke, \
                     patch.object(identity,'_observed_build',return_value='built') as build:
                    self.assertEqual(release.public('release-preflight'),'published')
                    with release.producer('android-native') as current:self.assertEqual(current,('fixture',{}))
                    self.assertEqual(observe.invoke(Path('/unused-helper'),[]),'observed')
                    self.assertEqual(identity.observed_build(Path('/unused-source'),{}),'built')
                    public.assert_called_once();invoke.assert_called_once();build.assert_called_once()
            finally:os.close(fd)

    def test_real_producer_error_keeps_its_reason_and_runs_cleanup(self):
        events=[]
        @contextlib.contextmanager
        def producer(_kind):
            try:yield ('fixture',{})
            finally:events.append('cleanup')
        with patch.object(release,'_producer',side_effect=producer):
            with self.assertRaisesRegex(ValueError,'underlying tool failed'):
                with release.producer('android-native'):raise ValueError('underlying tool failed')
        self.assertEqual(events,['cleanup'])
