#!/usr/bin/env python3
"""Signed-bundle adapter for the existing independent macOS upgrade transaction.

The native app verifies its bundle and exact signer before starting this module.
This module only selects and verifies a full signed release, then delegates all
installation, idle checks, and recovery semantics to the existing installer.
"""
import hashlib
import io
import json
import os
from pathlib import Path
import re
import sys
import tempfile
import urllib.error
import urllib.request

import desktop_identity as desktop
import install_macos_desktop as fresh
import macos_release_guard as guard
import upgrade_macos_desktop as upgrade
# The transaction imports these after staging. Load from the verified old bundle
# before the application can be exchanged with the new bundle.
import desktop_upgrade_rollback  # noqa: F401
import migrate_macos_desktop_signer  # noqa: F401

BASE = 'https://weavelo.hackerlife.fun:8443/download/'
MAX_MANIFEST = 128 * 1024
MAX_ARCHIVE = 1024 ** 3
PREFIX = 'Weavelo-desktop-macos-arm64-'

class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise guard.ReleaseRejected('Release server redirect refused')


def metadata(data, expected_signer=None):
    try:
        value = json.loads(data)
        if value['schema'] != 1 or not isinstance(value['releases'], list):
            raise ValueError('schema')
        entries = [e for e in value['releases'] if isinstance(e, dict)
                   and isinstance(e.get('file'), str) and e['file'].startswith(PREFIX)
                   and e['file'].endswith('.zip') and re.fullmatch(r'[A-Za-z0-9._-]+', e['file'])]
        if len(entries) != 1: raise ValueError('package selection')
        item = entries[0]
        pins = item['build_identity_expectations']
        if not isinstance(pins, dict) or set(pins) != {'commit', 'identity_sha256', 'manifest_sha256'}:
            raise ValueError('identity')
        for key, length in [('commit', 40), ('identity_sha256', 64), ('manifest_sha256', 64)]:
            if not isinstance(pins[key], str) or not re.fullmatch('[0-9a-f]{%d}' % length, pins[key]):
                raise ValueError('identity')
        if item.get('source_commit') != pins['commit'] or not isinstance(item.get('sha256'), str) or not re.fullmatch('[0-9a-f]{64}', item['sha256']):
            raise ValueError('digest')
        if expected_signer is not None:
            if item.get('signer_sha1') != expected_signer:
                raise ValueError('signer')
            guard.build_key(item.get('build', ''))
        size = item.get('size_bytes')
        if type(size) is not int or not 0 < size <= MAX_ARCHIVE: raise ValueError('size')
        return item
    except (KeyError, TypeError, ValueError, json.JSONDecodeError) as error:
        raise guard.ReleaseRejected('Invalid or ambiguous macOS release metadata') from error


def download(opener, url, output, limit, expected_size=None):
    digest = hashlib.sha256()
    total = 0
    with opener.open(urllib.request.Request(url, headers={'Cache-Control': 'no-cache'}), timeout=30) as response:
        while True:
            block = response.read(min(1024 * 1024, limit + 1 - total))
            if not block: break
            total += len(block)
            if total > limit: raise guard.ReleaseRejected('Release download exceeds its size limit')
            digest.update(block)
            output.write(block)
    if expected_size is not None and total != expected_size:
        raise guard.ReleaseRejected('Release download size differs from metadata')
    return digest.hexdigest()


def fetch_metadata(opener, signer):
    if not re.fullmatch(r'[0-9a-f]{40}', signer):
        raise guard.ReleaseRejected('Invalid installed signer')
    data = io.BytesIO()
    try:
        download(opener, BASE + 'desktop-releases-macos-' + signer + '.json', data, MAX_MANIFEST)
    except urllib.error.HTTPError as error:
        code = error.code
        error.close()
        if code != 404: raise
        data = io.BytesIO()
        download(opener, BASE + 'desktop-releases.json', data, MAX_MANIFEST)
        return metadata(data.getvalue()), False
    return metadata(data.getvalue(), signer), True


def release_decision(previous, candidate, previous_tree, candidate_tree):
    decision = guard.check_upgrade(previous, candidate)
    if decision == 'already_installed' and previous_tree == candidate_tree:
        return decision
    if guard.build_key(candidate['build']) <= guard.build_key(previous['build']):
        raise guard.ReleaseRejected('Published package is not newer than the installed build')
    return decision


def run():
    if sys.platform != 'darwin': raise guard.ReleaseRejected('macOS required')
    home = Path.home().resolve(strict=True)
    app = desktop.paths(home)['app']
    root = Path(__file__).resolve(strict=True).parent.parent
    if root != app / 'Contents/Resources/desktop-updater':
        raise guard.ReleaseRejected('Updater must run from the installed application')
    policy = guard.desktop_policy(app)
    context = upgrade.inspect(home)
    if context['signed']['certificate_sha1'] != policy.certificate_sha1:
        raise guard.ReleaseRejected('Installed signer changed')
    opener = urllib.request.build_opener(NoRedirect())
    with tempfile.TemporaryDirectory(prefix='weavelo-update-') as directory:
        directory = Path(directory).resolve(strict=True)
        os.chmod(directory, 0o700)
        item, signer_specific = fetch_metadata(opener, policy.certificate_sha1)
        archive = directory / 'candidate.zip'
        with archive.open('xb') as output:
            digest = download(opener, BASE + item['file'], output, item['size_bytes'], item['size_bytes'])
        if digest != item['sha256']: raise guard.ReleaseRejected('Release download digest differs from metadata')
        pins = item['build_identity_expectations']
        try:
            with guard.verified_archive(archive, policy, identity_expectations=pins) as (candidate, signed):
                fresh.verified_product(candidate, archive, pins)
                if signer_specific and signed['build'] != item['build']:
                    raise guard.ReleaseRejected('Signed package build differs from release metadata')
                decision = release_decision(context['signed'], signed, context['tree'], upgrade.tree_digest(candidate))
        except Exception as error:
            raise guard.ReleaseRejected('Published package failed signing or identity verification: ' + str(error)) from error
        if decision == 'already_installed':
            return {'outcome': decision, 'applied': False}
        return upgrade.apply(archive, home, pins, digest, context['pins'],
                             no_new_rollback=True, idle_only=True)


def main():
    try:
        result = run()
        print(json.dumps({'ok': True, 'outcome': result.get('outcome', result.get('phase', 'completed'))}))
    except Exception as error:
        print(json.dumps({'ok': False, 'error': str(error)[:1000]}))
        return 1
    return 0

if __name__ == '__main__':
    sys.exit(main())
