#!/usr/bin/env python3
"""One private same-signer Desktop rollback slot. No credentials/profile copying.

The normal upgrader prepares this verified slot before stopping any owner. An
explicit rollback pins the slot, actual bundle tuple and current registry. Unknown
or partial records are retained; no forward replay or broad cleanup is provided.
"""
import argparse
from contextlib import contextmanager
import fcntl
import json
import os
from pathlib import Path
import shutil
import sys

import upgrade_macos_desktop as up
import migrate_macos_desktop_signer as native

KIND = 'weavelo-desktop-same-signer-rollback-v1'
require = up.require


def encoded(value):
    return (json.dumps(value, indent=2) + '\n').encode()


def slot(selected):
    return selected['app'].parent / '.desktop-upgrade.rollback'


def prepare(context, candidate_pins, candidate_tree, candidate_signed, archive_sha, before):
    p = context['paths']; root = slot(p)
    value = {'schema': 1, 'kind': KIND, 'paths': {k: str(p[k]) for k in ('app', 'profile', 'state', 'plist')},
             'current': context['pins'], 'old_source': context['record']['source'],
             'old_tree': context['tree'], 'new_tree': candidate_tree, 'source': candidate_pins,
             'archive_sha256': archive_sha, 'new_executable_sha256': candidate_signed['executable_sha256'],
             'before': native.runtime_record(before)}
    # A completed slot never gets overwritten by a subsequent update. The caller
    # must inspect it; this implementation does not prune any prior rollback.
    if root.exists() or root.is_symlink():
        up.private(root, True)
        found, _ = up.read_record(root / 'rollback.json')
        require(found == value, 'Existing rollback slot differs; preserve and inspect it')
        verify_backups(p, found, old=True)
        return value
    root.mkdir(mode=0o700); up.fsync(root.parent)
    for name, source in (('install.before.json', p['state'] / 'install.json'),
                         ('launch-agent.before.plist', p['plist'])):
        native.write_new(root / name, source.read_bytes())
    shutil.copytree(p['app'], root / 'Weavelo.app', symlinks=False)
    up.fsync_tree(root / 'Weavelo.app')
    native.write_new(root / 'rollback.json', encoded(value))
    verify_backups(p, value, old=True)
    return value


def verify_backups(p, value, *, old):
    root = slot(p); up.private(root, True)
    require(value.get('schema') == 1 and value.get('kind') == KIND
            and value['paths'] == {k: str(p[k]) for k in ('app', 'profile', 'state', 'plist')},
            'Rollback profile/path binding differs')
    require(up.gate.hash_file(root / 'install.before.json') == value['current']['install']
            and up.gate.hash_file(root / 'launch-agent.before.plist') == value['current']['plist']
            and up.gate.hash_file(p['plist']) == value['current']['plist'], 'Original rollback metadata changed')
    tree = value['old_tree'] if old else value['new_tree']
    require(up.tree_digest(root / 'Weavelo.app') == tree, 'Rollback bundle tuple changed')
    signed = up.guard.verify_app(root / 'Weavelo.app', up.guard.desktop_policy(p['app']))
    expected = value['current']['executable'] if old else value['new_executable_sha256']
    require(signed['executable_sha256'] == expected, 'Rollback executable changed')


@contextmanager
def profile_locked(p):
    root = p['profile']; before = up.private(root, True)
    fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        opened = os.fstat(fd)
        require((opened.st_dev, opened.st_ino) == (before.st_dev, before.st_ino), 'Profile directory changed')
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        now = up.private(root, True)
        require((now.st_dev, now.st_ino) == (opened.st_dev, opened.st_ino), 'Profile lock changed')
        yield
    finally:
        os.close(fd)


def no_pending_profile(p):
    entry = p['profile'] / 'mesh.secret-recovery'
    require(not entry.exists() and not entry.is_symlink(), 'Pending profile recovery must finish first')


def reconstructed_new(original, value, current):
    result = dict(original)
    previous = dict(original); previous.pop('history', None)
    result.update(history=[*original.get('history', []), previous], source=value['source'],
                  archive_sha256=value['archive_sha256'], phase='upgrade_completed', service_started=True, signed_application_verified=True,
                  launch_agent_preserved_sha256=value['current']['plist'])
    runtime = current.get('runtime')
    require(isinstance(runtime, dict) and runtime.get('executable_sha256') == value['new_executable_sha256'],
            'Updated runtime receipt differs')
    result['runtime'] = runtime
    require(result == current, 'Installation metadata changed since this upgrade; nothing restored')
    return result


def observe(home):
    p = up.owned_layout(home); root = slot(p)
    value, record_sha = up.read_record(root / 'rollback.json')
    original, _ = up.read_record(root / 'install.before.json')
    current, registry_sha = up.read_record(p['state'] / 'install.json')
    active_tree = up.tree_digest(p['app'])
    require(active_tree in (value['old_tree'], value['new_tree']), 'Unknown active bundle tuple')
    side = 'old' if active_tree == value['old_tree'] else 'new'
    verify_backups(p, value, old=side == 'new')
    signed = up.guard.verify_app(p['app'], up.guard.desktop_policy(p['app']))
    expected = value['current']['executable'] if side == 'old' else value['new_executable_sha256']
    require(signed['executable_sha256'] == expected, 'Active executable differs')
    if registry_sha != value['current']['install']:
        reconstructed_new(original, value, current)
    # A pending profile transaction needs the new core's recovery, never an
    # automatic rollback to a binary which cannot interpret that pending state.
    no_pending_profile(p)
    control = up.NativeControl(p)
    before = control.before(expected)
    runtime = native.runtime_record(before)
    result = {'schema': 1, 'kind': KIND, 'slot_record_sha256': record_sha,
              'active_side': side, 'registry_sha256': registry_sha, 'active_executable_sha256': expected,
              'membership': up.membership_metadata(p['profile']), 'runtime': runtime}
    return p, value, result, before


def plan(home):
    _, _, value, _ = observe(home)
    return dict(value, plan_sha256=up.gate.digest(up.gate.canonical(value)), applied=False)


def rollback(home, expected_plan):
    p = up.owned_layout(home)
    lock = p['state'].parent / '.desktop-install.lock'
    fd = os.open(lock, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'r+') as held:
        up.private(lock)
        require(os.fstat(held.fileno()).st_ino == lock.lstat().st_ino, 'Installer lock changed')
        fcntl.flock(held, fcntl.LOCK_EX | fcntl.LOCK_NB)
        p, value, state, before = observe(home)
        require(up.gate.digest(up.gate.canonical(state)) == up.gate.sha(expected_plan), 'Rollback plan changed')
        journal = slot(p) / 'restore.json'
        require(not journal.exists() and not journal.is_symlink(), 'Rollback already started; inspect original status')
        # Every irreversible boundary is journaled, but an interrupted rollback
        # is deliberately not replayed by this entry. Actual tuple stays visible.
        data = {'schema': 1, 'kind': KIND, 'plan': state, 'phase': 'stopping'}
        sha = native.write_new(journal, encoded(data))
        sha = up.gate.hash_file(journal)
        control = up.NativeControl(p); control.stop(before)
        def checkpoint(phase):
            nonlocal sha
            data['phase'] = phase; sha = up.atomic_record(journal, data, sha)
        checkpoint('stopped')
        with up.idle_locks(p['profile']), profile_locked(p):
            no_pending_profile(p)
            require(up.gate.hash_file(slot(p) / 'rollback.json') == state['slot_record_sha256'], 'Rollback record changed')
            verify_backups(p, value, old=state['active_side'] == 'new')
            require(up.membership_metadata(p['profile']) == state['membership'], 'Membership changed during quit')
            require(up.tree_digest(p['app']) == (value['old_tree'] if state['active_side'] == 'old' else value['new_tree']),
                    'Bundle changed before rollback')
            require(up.gate.hash_file(p['state'] / 'install.json') == state['registry_sha256'], 'Registry changed before rollback')
            if state['active_side'] == 'new':
                up.fsync_tree(p['app']); up.fsync_tree(slot(p) / 'Weavelo.app')
                checkpoint('exchanging'); up.exchange(slot(p) / 'Weavelo.app', p['app'])
            checkpoint('exchanged')
            require(up.tree_digest(p['app']) == value['old_tree'], 'Restored bundle did not verify')
            require(up.gate.hash_file(p['state'] / 'install.json') == state['registry_sha256'], 'Registry changed')
            if state['registry_sha256'] != value['current']['install']:
                raw = (slot(p) / 'install.before.json').read_bytes()
                require(up.gate.digest(raw) == value['current']['install'], 'Original registry bytes changed')
                up.atomic_bytes(p['state'] / 'install.json', raw, state['registry_sha256'], suffix='.desktop-restore.pending')
        checkpoint('registry_restored')
        native.restore_runtime(control, state['runtime'], value['current']['executable'])
        require(up.membership_metadata(p['profile']) == state['membership']
                and up.gate.hash_file(p['plist']) == value['current']['plist'], 'Membership or launch policy changed during restart')
        require(up.tree_digest(p['app']) == value['old_tree']
                and up.gate.hash_file(p['state'] / 'install.json') == value['current']['install'],
                'Restored bundle or registry changed during restart')
        checkpoint('completed')
        return dict(data, applied=True, membership_preserved=True, failed_candidate_retained=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apply', action='store_true')
    parser.add_argument('--expected-plan-sha256')
    args = parser.parse_args()
    require(sys.platform == 'darwin', 'Native macOS required')
    if args.apply:
        require(args.expected_plan_sha256, 'Explicit observed rollback plan required')
        result = rollback(Path.home(), args.expected_plan_sha256)
    else:
        require(not args.expected_plan_sha256, 'Read-only plan takes no mutation pins')
        result = plan(Path.home())
    print(json.dumps(result, indent=2))


if __name__ == '__main__': main()
