#!/usr/bin/env python3
"""One protected Desktop credential checkpoint, never an invitation export.

Uses the existing Linux app-private credential backend and profile-directory lock.
Restore only stages the core's existing v1 rollback record with owners stopped;
normal core startup performs the verified backward recovery. No key is printed,
hashed into a public receipt, moved outside the backend or put in arguments.
"""
import argparse
import errno
from contextlib import contextmanager, ExitStack
import fcntl
import hashlib
import json
import os
from pathlib import Path
import stat
import sys
import uuid

from install_linux_desktop import exclusive_rename

LIMIT = 1024 * 1024
CHECKPOINT = 'mesh.secret-checkpoint'
PENDING = 'mesh.secret-recovery'
# Existing private CLI accepts at most 16 KiB; reserve worst-case escaped
# invitation-field growth before declaring an original checkpoint restorable.
RESTORE_RESERVE = 8 * 16 * 1024


class Refused(RuntimeError): pass


def require(ok, text):
    if not ok: raise Refused(text)


# Rust str::trim uses Unicode White_Space, not Python's larger isspace set.
RUST_WHITESPACE = "\t\n\v\f\r \u0085\u00a0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200a\u2028\u2029\u202f\u205f\u3000"


def rust_trim(value): return value.strip(RUST_WHITESPACE)


def no_extra_acl(path):
    # This checkpoint adapter is intentionally Linux-only. Do not infer absent
    # macOS ACLs from ambiguous native ACL error returns or weaken that check.
    require(sys.platform.startswith('linux'), 'Linux credential checkpoint adapter required')
    require(not {'system.posix_acl_access', 'system.posix_acl_default'} & set(os.listxattr(path)),
            'Nontrivial ACL requires its own protection adapter; nothing copied')


def label(path):
    if not sys.platform.startswith('linux'): return None
    try: return os.getxattr(path, 'security.selinux')
    except OSError as error:
        if error.errno in (errno.ENODATA, errno.ENOTSUP): return None
        raise Refused('Security label metadata unavailable; nothing copied') from None


def private(path, directory=False):
    path = Path(path)
    info = path.lstat()
    require((stat.S_ISDIR(info.st_mode) if directory else stat.S_ISREG(info.st_mode))
            and info.st_uid == os.getuid() and not info.st_mode & 0o077
            and (directory or info.st_nlink == 1), 'Private owned backend entry required')
    no_extra_acl(path)
    return info


def read(path):
    info = private(path)
    require(info.st_size <= LIMIT, 'Protected backend entry exceeds bound')
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        opened = os.fstat(fd)
        require((opened.st_dev, opened.st_ino) == (info.st_dev, info.st_ino), 'Backend entry changed')
        data = b''
        while len(data) <= LIMIT:
            block = os.read(fd, LIMIT + 1 - len(data))
            if not block: break
            data += block
        now = private(path)
        require(len(data) <= LIMIT and (now.st_dev, now.st_ino, now.st_size, now.st_mtime_ns)
                == (opened.st_dev, opened.st_ino, opened.st_size, opened.st_mtime_ns), 'Backend entry changed')
        return data.decode('utf-8')
    finally: os.close(fd)


@contextmanager
def locked(root):
    root = Path(root)
    require(os.getuid() != 0 and root.is_absolute() and root == root.resolve(), 'Canonical ordinary-user profile required')
    for parent in root.parents:
        info = parent.stat()
        trusted_tmp = info.st_uid == 0 and bool(info.st_mode & stat.S_ISVTX)
        require(not parent.is_symlink() and parent.is_dir() and info.st_uid in (0, os.getuid())
                and (not info.st_mode & 0o022 or trusted_tmp), 'Unsafe profile ancestor')
    before = private(root, True)
    fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        require((os.fstat(fd).st_dev, os.fstat(fd).st_ino) == (before.st_dev, before.st_ino), 'Profile changed')
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        require(root.stat().st_ino == before.st_ino, 'Profile lock changed')
        yield
    finally: os.close(fd)


@contextmanager
def stopped(root):
    with ExitStack() as held:
        private(root / 'gui-state', True)
        for path in (root / 'desktop-instance.lock', root / 'gui-state/desktop-instance.lock'):
            info = private(path)
            fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
            held.callback(os.close, fd)
            require(os.fstat(fd).st_ino == info.st_ino, 'Lifecycle lock changed')
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield


def fields(metadata):
    entries = {}
    for raw_line in metadata.split('\n'):
        line = rust_trim(raw_line)
        if not line or line.startswith('#'): continue
        require('=' in line, 'Invalid public profile metadata')
        key, value = line.split('=', 1)
        if key == 'version': entries[key] = value
        elif key in ('network_name_hex', 'node_id_hex', 'display_name_hex'):
            require(len(value) % 2 == 0 and all(c in '0123456789abcdefABCDEF' for c in value),
                    'Invalid public metadata hex')
            entries[key] = bytes.fromhex(value).decode('utf-8')
    require(entries.get('version') in ('weavelo-device-group-v2', 'weavelo-mesh-v1'), 'Unsupported profile metadata')
    result = {key: entries[key + '_hex'] for key in ('network_name', 'node_id', 'display_name')}
    require(all(rust_trim(value) for value in result.values()), 'Invalid public profile metadata')
    return result


def public(metadata):
    data = fields(metadata)
    return {'network_name_sha256': hashlib.sha256(data['network_name'].encode()).hexdigest(),
            'device_id_sha256': hashlib.sha256(data['node_id'].encode()).hexdigest()}


def pair(root):
    metadata = read(root / 'mesh.conf'); fields(metadata)
    secret = rust_trim(read(root / 'mesh.secret'))
    require(bool(secret), 'Existing credential is missing')
    return {'metadata': metadata, 'secret': secret}


def no_pending(root):
    path = root / PENDING
    require(not path.exists() and not path.is_symlink(), 'Pending core recovery retained; do not repeat')


def _boundary(_name): pass  # Patched only in isolated tests, never an environment-controlled failpoint.


def write_new(path, value):
    raw = (json.dumps(value, sort_keys=True, ensure_ascii=True) + '\n').encode()
    require(len(raw) <= LIMIT, 'Protected record exceeds bound; nothing written')
    require(not path.exists() and not path.is_symlink(), 'Existing protected record retained')
    pending = path.with_name('.' + path.name + '.pending')
    fd = os.open(pending, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    try:
        private(pending)
        require(label(pending) == label(path.parent / 'mesh.secret'), 'Credential security label differs; nothing copied')
        _boundary('created')
        offset = 0
        while offset < len(raw):
            n = os.write(fd, raw[offset:]); require(n > 0, 'Protected write stalled'); offset += n
        _boundary('written'); os.fsync(fd); _boundary('synced')
    finally: os.close(fd)
    exclusive_rename(pending, path); _boundary('published')
    directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try: os.fsync(directory)
    finally: os.close(directory)
    _boundary('directory-synced')
    private(path)
    require(label(path) == label(path.parent / 'mesh.secret'), 'Published credential security label differs; retained')
    require(read(path).encode() == raw, 'Protected record did not verify; retained')


def create(root, expected):
    root = Path(root)
    with locked(root), stopped(root):
        no_pending(root)
        before = pair(root)
        require(public(before['metadata']) == expected, 'Public source identity changed')
        reverse = {'version': 1, 'profile': str(root), 'before_metadata': before['metadata'],
                   'before_secret': before['secret'], 'after_metadata': before['metadata'], 'after_secret': before['secret']}
        require(len(json.dumps(reverse, ensure_ascii=True).encode()) + RESTORE_RESERVE <= LIMIT,
                'Core rollback budget is unavailable; nothing copied')
        value = {'schema': 1, 'kind': 'weavelo-private-profile-checkpoint', 'profile': str(root),
                 'checkpoint_id': str(uuid.uuid4()), 'pair': before}
        write_new(root / CHECKPOINT, value)
        return {'ok': True, 'checkpoint_id': value['checkpoint_id'], 'public_identity': expected,
                'protection': 'existing-app-private-credential-backend', 'invitation_exported': False}


def checkpoint(root, identifier):
    private(root / 'mesh.secret')
    private(root / CHECKPOINT)
    require(label(root / CHECKPOINT) == label(root / 'mesh.secret'), 'Checkpoint security label differs; retained')
    value = json.loads(read(root / CHECKPOINT))
    require(set(value) == {'schema', 'kind', 'profile', 'checkpoint_id', 'pair'}
            and type(value['schema']) is int and value['schema'] == 1 and value['kind'] == 'weavelo-private-profile-checkpoint'
            and value['profile'] == str(root) and (identifier is None or value['checkpoint_id'] == identifier)
            and str(uuid.UUID(value['checkpoint_id'])) == value['checkpoint_id'],
            'Checkpoint/profile binding differs')
    require(set(value['pair']) == {'metadata', 'secret'} and isinstance(value['pair']['secret'], str)
            and rust_trim(value['pair']['secret']), 'Checkpoint credential is invalid')
    fields(value['pair']['metadata'])
    return value


def stage_restore(root, identifier, expected):
    root = Path(root)
    with locked(root), stopped(root):
        no_pending(root)
        saved = checkpoint(root, identifier)['pair']; current = pair(root)
        require(public(current['metadata']) == expected, 'Current public identity changed')
        old_fields, now_fields = fields(saved['metadata']), fields(current['metadata'])
        require((old_fields['node_id'], old_fields['display_name']) == (now_fields['node_id'], now_fields['display_name']),
                'Device identity or name changed; nothing overwritten')
        if saved == current:
            return {'ok': True, 'already_original': True, 'public_identity': public(saved['metadata'])}
        # This is the existing core rollback wire contract. Its before is the
        # saved original, after is the independently read current tuple. The core
        # must match both before performing any backward recovery.
        record = {'version': 1, 'profile': str(root), 'before_metadata': saved['metadata'],
                  'before_secret': saved['secret'], 'after_metadata': current['metadata'], 'after_secret': current['secret']}
        write_new(root / PENDING, record)
        return {'ok': True, 'restore_staged': True, 'checkpoint_id': identifier,
                'public_identity': public(saved['metadata']), 'core_startup_required': True,
                'forward_replay_supported': False}


def inspect(root):
    root = Path(root)
    with locked(root):
        path = root / CHECKPOINT
        value = checkpoint(root, None) if path.exists() or path.is_symlink() else None
        pending = root / PENDING
        incomplete = root / ('.' + CHECKPOINT + '.pending')
        restore_incomplete = root / ('.' + PENDING + '.pending')
        return {'ok': True, 'checkpoint_present': value is not None,
                'checkpoint_id': value['checkpoint_id'] if value else None,
                'public_identity': public(value['pair']['metadata']) if value else None,
                'core_recovery_pending': pending.exists() or pending.is_symlink(),
                'unknown_checkpoint_pending': incomplete.exists() or incomplete.is_symlink(),
                'unknown_restore_pending': restore_incomplete.exists() or restore_incomplete.is_symlink(),
                'invitation_exported': False}


def default_profile():
    home = Path.home().resolve()
    return home / '.config/weavelo/desktop'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('inspect', 'create', 'stage-restore'))
    parser.add_argument('--expected-network-name-sha256')
    parser.add_argument('--expected-device-id-sha256')
    parser.add_argument('--checkpoint-id')
    args = parser.parse_args()
    require(sys.platform.startswith('linux'), 'Existing Linux credential backend only')
    if args.action == 'inspect':
        require(not args.checkpoint_id and not args.expected_network_name_sha256 and not args.expected_device_id_sha256, 'Inspect takes no mutation pins')
        print(json.dumps(inspect(default_profile()))); return
    expected = {'network_name_sha256': args.expected_network_name_sha256, 'device_id_sha256': args.expected_device_id_sha256}
    require(all(isinstance(value, str) and len(value) == 64 and all(c in '0123456789abcdef' for c in value) for value in expected.values()), 'Public pins required')
    if args.action == 'create':
        require(args.checkpoint_id is None, 'Create does not replay a checkpoint')
        result = create(default_profile(), expected)
    else:
        require(bool(args.checkpoint_id), 'Original checkpoint identifier required')
        result = stage_restore(default_profile(), args.checkpoint_id, expected)
    print(json.dumps(result))


if __name__ == '__main__':
    try: main()
    except (Refused, OSError, ValueError, KeyError, TypeError):
        print(json.dumps({'ok': False, 'error': 'protected_profile_action_refused',
                          'next_action': 'Inspect original protected state; do not replay an uncertain action'}))
        sys.exit(1)
