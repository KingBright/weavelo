"""Exercise the existing NAS receiver in a local temp folder: zero remote calls."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest


class ReceiverTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='rp-nas-receiver-fixture-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name); self.stage = self.root / 'stage'; self.stage.mkdir()
        self.target = self.root / 'receiver'
        script = (Path(__file__).parents[1] / 'publish_desktop_nas.sh').read_text()
        begin = script.index("<<'PY'", script.index('ssh -p')) + len("<<'PY'\n")
        self.code = script[begin:script.index('\nPY\n', begin)]
        releases = []
        for name in ('macos.zip', 'linux.tar.gz', 'windows.zip'):
            p = self.stage / name; p.write_bytes(('own fixture ' + name).encode())
            releases.append({'file': name, 'sha256': hashlib.sha256(p.read_bytes()).hexdigest()})
        (self.stage / 'desktop-releases.json').write_text(json.dumps({'releases': releases}))
        (self.stage / 'acceptance-2.0.0-alpha.2.html').write_text('own fixture')
        (self.stage / 'index.html').write_text('own fixture')

    def receive(self, scope=None):
        archive = self.root / 'incoming.tar'
        with tarfile.open(archive, 'w') as tar:
            for p in self.stage.iterdir(): tar.add(p, arcname=p.name)
        return subprocess.run([sys.executable, '-B', '-c', self.code, str(archive), str(self.target)] + ([scope] if scope else []), capture_output=True, text=True)

    def test_receiver_only_installs_finished_files_and_same_bytes_are_noop(self):
        first = self.receive(); self.assertEqual(first.returncode, 0, first.stderr)
        self.assertIn('INSTALLED', first.stdout)
        times = {p.name: p.stat().st_mtime_ns for p in self.target.iterdir()}
        second = self.receive(); self.assertEqual(second.returncode, 0, second.stderr)
        self.assertNotIn('INSTALLED', second.stdout)
        self.assertEqual(times, {p.name: p.stat().st_mtime_ns for p in self.target.iterdir()})

    def test_same_filename_with_changed_bytes_stops_before_metadata_update(self):
        self.assertEqual(self.receive().returncode, 0)
        original = (self.target / 'desktop-releases.json').read_bytes()
        p = self.stage / 'macos.zip'; p.write_bytes(b'different')
        data = json.loads((self.stage / 'desktop-releases.json').read_text())
        data['releases'][0]['sha256'] = hashlib.sha256(p.read_bytes()).hexdigest()
        (self.stage / 'desktop-releases.json').write_text(json.dumps(data))
        failed = self.receive(); self.assertNotEqual(failed.returncode, 0)
        self.assertIn('immutable artifact conflict', failed.stderr)
        self.assertEqual(original, (self.target / 'desktop-releases.json').read_bytes())

    def test_receiver_has_no_build_or_source_sync_command(self):
        for command in ('cargo', 'rustc', 'gradle', 'zigbuild', 'docker build', 'npm', 'pnpm'):
            self.assertNotIn(command, self.code)

    def prepare_macos_update(self):
        doc = json.loads((self.stage / 'desktop-releases.json').read_text())
        doc['acceptance_file'] = 'acceptance-2.0.0-alpha.2.html'
        (self.stage / 'desktop-releases.json').write_text(json.dumps(doc))
        (self.stage / 'index.html').write_text('Android unchanged; <a href="' + doc['acceptance_file'] + '">notes</a>')
        self.assertEqual(self.receive().returncode, 0)
        before = (self.target / 'desktop-releases.json').read_bytes()
        (self.stage / 'desktop-releases.before.json').write_bytes(before)
        (self.stage / 'index.before.html').write_bytes((self.target / 'index.html').read_bytes())
        for entry in doc['releases']:
            (self.stage / entry['file']).unlink()
        (self.stage / doc['acceptance_file']).unlink()
        package = self.stage / 'macos-new.zip'; package.write_bytes(b'new own fixture')
        doc['releases'][0] = {'file': package.name, 'version': 'fixture.2', 'size_bytes': package.stat().st_size,
                              'sha256': hashlib.sha256(package.read_bytes()).hexdigest(),
                              'build_identity_expectations': {'commit': 'a' * 40, 'identity_sha256': 'b' * 64,
                                                              'manifest_sha256': 'c' * 64}}
        doc['acceptance_file'] = 'acceptance-macos-new.html'
        (self.stage / doc['acceptance_file']).write_text('Only fixture Mac changed; other platforms retained.')
        (self.stage / 'desktop-releases.json').write_text(json.dumps(doc))
        (self.stage / 'index.html').write_bytes((self.stage / 'index.before.html').read_bytes().replace(
            b'acceptance-2.0.0-alpha.2.html', b'acceptance-macos-new.html'))
        return json.loads(before)

    def test_macos_update_preserves_other_entries_bytes_and_rollback(self):
        before = self.prepare_macos_update()
        (self.target / 'release.json').write_bytes(b'unrelated Android metadata')
        preserved = {p.name: (p.read_bytes(), p.stat().st_mtime_ns) for p in self.target.iterdir()
                     if p.name not in ('desktop-releases.json', 'index.html', '.desktop-publish.lock')}
        result = self.receive('macos'); self.assertEqual(result.returncode, 0, result.stderr)
        after = json.loads((self.target / 'desktop-releases.json').read_text())
        self.assertEqual(after['releases'][1:], before['releases'][1:])
        self.assertEqual(preserved, {name: ((self.target/name).read_bytes(), (self.target/name).stat().st_mtime_ns)
                                     for name in preserved})
        self.assertEqual((self.target / 'macos-new.zip').read_bytes(), b'new own fixture')

    def test_macos_update_rejects_changes_to_another_platform_before_writing(self):
        self.prepare_macos_update()
        original = (self.target / 'desktop-releases.json').read_bytes()
        doc = json.loads((self.stage / 'desktop-releases.json').read_text())
        doc['releases'][1]['version'] = 'invented update'
        (self.stage / 'desktop-releases.json').write_text(json.dumps(doc))
        result = self.receive('macos'); self.assertNotEqual(result.returncode, 0)
        self.assertIn('other platform entries changed', result.stderr)
        self.assertEqual((self.target / 'desktop-releases.json').read_bytes(), original)
        self.assertFalse((self.target / 'macos-new.zip').exists())

    def test_macos_update_rejects_stale_baseline_before_writing(self):
        self.prepare_macos_update()
        original = (self.target / 'desktop-releases.json').read_bytes() + b'\n'
        (self.target / 'desktop-releases.json').write_bytes(original)
        result = self.receive('macos'); self.assertNotEqual(result.returncode, 0)
        self.assertIn('publication changed since read', result.stderr)
        self.assertEqual((self.target / 'desktop-releases.json').read_bytes(), original)
        self.assertFalse((self.target / 'macos-new.zip').exists())

    def test_macos_update_rejects_changes_to_unrelated_index_content(self):
        self.prepare_macos_update()
        original = (self.target / 'index.html').read_bytes()
        (self.stage / 'index.html').write_text('replaced Android download page')
        result = self.receive('macos'); self.assertNotEqual(result.returncode, 0)
        self.assertIn('unrelated index content changed', result.stderr)
        self.assertEqual((self.target / 'index.html').read_bytes(), original)
        self.assertFalse((self.target / 'macos-new.zip').exists())

    def test_macos_update_checks_preserved_package_bytes_before_writing(self):
        self.prepare_macos_update()
        (self.target / 'linux.tar.gz').write_bytes(b'changed behind the manifest')
        original = (self.target / 'desktop-releases.json').read_bytes()
        result = self.receive('macos'); self.assertNotEqual(result.returncode, 0)
        self.assertIn('checksum mismatch', result.stderr)
        self.assertEqual((self.target / 'desktop-releases.json').read_bytes(), original)
        self.assertFalse((self.target / 'macos-new.zip').exists())

    def test_macos_update_preserves_an_existing_conflicting_acceptance_page(self):
        self.prepare_macos_update()
        (self.target / 'acceptance-macos-new.html').write_bytes(b'existing different page')
        result = self.receive('macos'); self.assertNotEqual(result.returncode, 0)
        self.assertIn('immutable acceptance conflict', result.stderr)
        self.assertEqual((self.target / 'acceptance-macos-new.html').read_bytes(), b'existing different page')
        self.assertFalse((self.target / 'macos-new.zip').exists())


    def prepare_signer(self, signer, build, name, content):
        for path in self.stage.iterdir(): path.unlink()
        archive = self.stage / name; archive.write_bytes(content)
        entry = {'file': name, 'sha256': hashlib.sha256(content).hexdigest(), 'size_bytes': len(content),
                 'signer_sha1': signer, 'build': build, 'source_commit': 'a' * 40,
                 'build_identity_expectations': {'commit': 'a' * 40}}
        (self.stage / 'desktop-releases.json').write_text(json.dumps({'schema': 1, 'releases': [entry]}))
        return entry

    def test_signer_publication_requires_guard_verified_build_and_signer(self):
        signer = 'a' * 40
        self.prepare_signer(signer, '20261109.9', 'Weavelo-desktop-macos-arm64-current.zip', b'current')
        script = (Path(__file__).parents[1] / 'publish_desktop_nas.sh').read_text()
        begin = script.index("<<'PREFLIGHT'") + len("<<'PREFLIGHT'\n")
        code = script[begin:script.index('\nPREFLIGHT\n', begin)]
        def preflight(signed):
            report = 'MACOS_RELEASE_VERIFIED ' + json.dumps([signed])
            return subprocess.run([sys.executable, '-B', '-c', code, str(self.stage), 'signer', report],
                                  capture_output=True, text=True)
        self.assertEqual(preflight({'certificate_sha1': signer, 'build': '20261109.9'}).returncode, 0)
        for bad in ({'certificate_sha1': 'b' * 40, 'build': '20261109.9'},
                    {'certificate_sha1': signer, 'build': '20261107.148'}):
            self.assertNotEqual(preflight(bad).returncode, 0)

    def test_signer_upload_uses_the_native_verified_snapshot(self):
        name = 'Weavelo-desktop-macos-arm64-current.zip'
        entry = self.prepare_signer('a' * 40, '20261109.9', name, b'verified bytes')
        script = (Path(__file__).parents[1] / 'publish_desktop_nas.sh').read_text()

        def run_stage(marker, *args):
            start = script.index(f"<<'{marker}'\n") + len(f"<<'{marker}'\n")
            return subprocess.run([sys.executable, '-B', '-c',
                                   script[start:script.index(f'\n{marker}\n', start)],
                                   *map(str, args)], capture_output=True, text=True)

        snapshot = self.root / 'snapshot'
        snapshot.mkdir()
        copied = run_stage('SNAPSHOT_STAGE', self.stage, snapshot, 'signer')
        self.assertEqual(copied.returncode, 0, copied.stderr)
        self.prepare_signer('a' * 40, '20261109.9', name, b'replacement')
        report = 'MACOS_RELEASE_VERIFIED ' + json.dumps([
            {'archive_sha256': entry['sha256'], 'size_bytes': entry['size_bytes']}
        ])
        archive = self.root / 'upload.tar'
        packed = run_stage('PACK_VERIFIED_STAGE', snapshot, archive, 'signer', report)
        self.assertEqual(packed.returncode, 0, packed.stderr)
        with tarfile.open(archive) as data:
            self.assertEqual(data.extractfile(name).read(), b'verified bytes')
        rejected = run_stage('PACK_VERIFIED_STAGE', self.stage, archive, 'signer', report)
        self.assertIn('upload bytes differ from native guard', rejected.stderr)
        self.assertNotEqual(rejected.returncode, 0)

    def test_generic_macos_publication_also_advances_its_signer_manifest(self):
        self.prepare_macos_update()
        manifest = json.loads((self.stage / 'desktop-releases.json').read_text())
        signer = 'a' * 40
        entry = dict(manifest['releases'][0], signer_sha1=signer, build='20261109.9')
        (self.stage / 'macos-signer-current.json').write_text(json.dumps({'schema': 1, 'releases': [entry]}))
        received = self.receive('macos')
        self.assertEqual(received.returncode, 0, received.stderr)
        current = self.target / ('desktop-releases-macos-' + signer + '.json')
        self.assertEqual(json.loads(current.read_bytes())['releases'], [entry])
        self.assertEqual(json.loads((self.target / 'desktop-releases.json').read_bytes()), manifest)

    def test_signer_manifests_are_separate_and_preserve_generic_publication(self):
        self.assertEqual(self.receive().returncode, 0)
        generic = (self.target / 'desktop-releases.json').read_bytes()
        index = (self.target / 'index.html').read_bytes()
        a, b = 'a' * 40, 'b' * 40
        first = self.prepare_signer(a, '20261109.9', 'macos-signer-a.zip', b'first')
        result = self.receive('signer'); self.assertEqual(result.returncode, 0, result.stderr)
        second = self.prepare_signer(b, '20261109.10', 'macos-signer-b.zip', b'second')
        result = self.receive('signer'); self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.target / 'desktop-releases.json').read_bytes(), generic)
        self.assertEqual((self.target / 'index.html').read_bytes(), index)
        self.assertEqual(json.loads((self.target / ('desktop-releases-macos-'+a+'.json')).read_bytes())['releases'], [first])
        self.assertEqual(json.loads((self.target / ('desktop-releases-macos-'+b+'.json')).read_bytes())['releases'], [second])

    def test_signer_manifest_rejects_downgrade_and_same_build_different_package(self):
        self.assertEqual(self.receive().returncode, 0)
        signer = 'a' * 40
        self.prepare_signer(signer, '20261109.9', 'macos-current.zip', b'current')
        self.assertEqual(self.receive('signer').returncode, 0)
        manifest = self.target / ('desktop-releases-macos-'+signer+'.json')
        old = manifest.read_bytes()
        for build, name in [('20261107.148', 'macos-older.zip'), ('20261109.9', 'macos-other.zip')]:
            self.prepare_signer(signer, build, name, name.encode())
            rejected = self.receive('signer')
            self.assertNotEqual(rejected.returncode, 0)
            self.assertEqual(manifest.read_bytes(), old)
            self.assertFalse((self.target / name).exists())
        self.prepare_signer(signer, '20271201.1', 'macos-newer.zip', b'newer')
        updated = self.receive('signer'); self.assertEqual(updated.returncode, 0, updated.stderr)
        self.assertEqual(json.loads(manifest.read_bytes())['releases'][0]['build'], '20271201.1')
        self.assertEqual((self.target / 'macos-current.zip').read_bytes(), b'current')

if __name__ == '__main__':
    unittest.main()
