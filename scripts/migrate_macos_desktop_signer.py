#!/usr/bin/env python3
"""One pinned Studio Desktop certificate transition, separate from normal upgrade.

Default: read-only plan. --apply requires its exact digest. Never forward-resume.
--rollback requires the exact original transaction digest and observed bundle tuple.
No signing, profile copying, TCC/ACL changes, force quit or security exceptions.
"""
import argparse
from contextlib import contextmanager
import fcntl
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys

import upgrade_macos_desktop as up
import macos_release_guard as guard

CONTRACT = Path(__file__).resolve().parents[1] / 'deploy/macos/desktop-signer-migration.json'
OLD_LEAF = '8eaa97365d97b8a987cdf1232430eddf9aba8ecf'
NEW_LEAF = 'd38bedce6830fb01240f97c11c7984514ab715c0'
KIND = 'studio-desktop-signer-transition-20261025'
PHASES = ('prepared', 'staged', 'stopping', 'stopped', 'exchanging', 'exchanged',
          'activating', 'activated', 'committing', 'completed', 'rollback_stopping',
          'rollback_stopped', 'rollback_exchanging', 'rollback_exchanged',
          'rollback_registry', 'rollback_activating', 'rolled_back')
require = up.require
gate = up.gate


def digest(value):
    return gate.digest(gate.canonical(value))


def encoded(value):
    return (json.dumps(value, indent=2) + '\n').encode()


def contract():
    data = gate.read_json(CONTRACT)
    require(data.get('schema') == 1 and data.get('kind') == KIND
            and data.get('bundle_id') == up.desktop.identity()['application_id']
            and data.get('old_certificate_sha1') == OLD_LEAF
            and data.get('new_certificate_sha1') == NEW_LEAF, 'Unexpected fixed signer transition')
    require(set(data['current']) == {'executable', 'info', 'tree', 'install', 'upgrade', 'plist'},
            'Incomplete old installation pins')
    for value in data['current'].values(): gate.sha(value)
    for key in ('old_source', 'source'):
        require(set(data[key]) == {'commit', 'identity_sha256', 'manifest_sha256'}, 'Unknown source pins')
        gate.sha(data[key]['commit'], 40)
        for name in ('identity_sha256', 'manifest_sha256'): gate.sha(data[key][name])
    for key in ('archive_sha256', 'new_executable_sha256', 'new_tree_sha256'): gate.sha(data[key])
    require(Path(data['home']).is_absolute() and type(data['uid']) is int, 'Fixed owner required')
    require(guard.build_key(data['new_build']) > guard.build_key(data['old_build']), 'Transition must advance build')
    return data


def policies(c):
    new = guard.load_policy(guard.DESKTOP_POLICY_FILE)
    require(new.bundle_id == c['bundle_id'] and new.certificate_sha1 == NEW_LEAF
            and new.identity_kind == 'studio-local-desktop', 'Target signing policy changed')
    old = guard.Policy(c['bundle_id'], 'weavelo', 'Weavelo', OLD_LEAF)
    return old, new


def layout(home, c):
    home = Path(home)
    require(home == Path(c['home']) and os.getuid() == c['uid'] and os.geteuid() == c['uid'],
            'Fixed logged-in Studio owner required')
    selected = up.owned_layout(home)
    selected = dict(selected, rollback=selected['app'].parent / '.desktop-signer-rollback',
                    journal=selected['state'] / 'signer-migration.json')
    return selected


def absent(path):
    require(not path.exists() and not path.is_symlink(), 'Prior transition state exists; inspect, never replay')


def source_context(home, c):
    p = layout(home, c); old, _ = policies(c)
    record, install_sha = up.read_record(p['state'] / 'install.json')
    previous, upgrade_sha = up.read_record(p['state'] / 'upgrade.json')
    require(record.get('application_id') == c['bundle_id'] and record.get('applied') is True
            and record.get('phase') == 'upgrade_completed' and record.get('source') == c['old_source']
            and previous.get('schema') == 1 and previous.get('phase') == 'completed',
            'Exact completed source installation required')
    for key in ('app', 'profile', 'plist', 'state'):
        require(record.get('paths', {}).get(key) == str(p[key]), 'Original installation path binding changed')
    require(gate.hash_file(p['plist']) == c['current']['plist'], 'Source launch policy pin conflict')
    up.validate_policy(plistlib.loads(p['plist'].read_bytes()), p)
    signed = guard.verify_app(p['app'], old)
    require(signed['build'] == c['old_build'], 'Unexpected source build')
    up.product_current(p['app'] / 'Contents/MacOS/weavelo', record, signed)
    actual = {'executable': signed['executable_sha256'], 'info': signed['info_sha256'],
              'tree': up.tree_digest(p['app']), 'install': install_sha, 'upgrade': upgrade_sha,
              'plist': gate.hash_file(p['plist'])}
    require(actual == c['current'], 'Source installation pin conflict')
    return p, record, previous, signed, up.membership_metadata(p['profile'])


def validate_candidate(candidate, signed, c):
    _, new = policies(c)
    verified = guard.verify_app(candidate, new)
    # verified_archive adds archive digest/size to the native app report.
    require(all(signed.get(key) == value for key, value in verified.items())
            and signed.get('archive_sha256') == c['archive_sha256']
            and signed['build'] == c['new_build']
            and signed['executable_sha256'] == c['new_executable_sha256']
            and up.tree_digest(candidate) == c['new_tree_sha256'], 'Candidate signature/build pin conflict')


def make_plan(home, c, candidate, signed):
    p = layout(home, c)
    absent(p['journal']); absent(p['rollback'])
    absent(p['app'].parent / '.desktop-upgrade.pending')
    p, record, previous, old, membership = source_context(home, c)
    validate_candidate(candidate, signed, c)
    require(guard.version_key(signed['version']) >= guard.version_key(old['version']), 'Version downgrade rejected')
    value = {'schema': 1, 'kind': KIND, 'contract_sha256': digest(c),
             'paths': {k: str(p[k]) for k in ('app', 'profile', 'plist', 'state', 'rollback', 'journal')},
             'current': c['current'], 'source': c['source'], 'archive_sha256': c['archive_sha256'],
             'old_requirement': policies(c)[0].requirement, 'new_requirement': policies(c)[1].requirement,
             'old_tree': c['current']['tree'], 'new_tree': c['new_tree_sha256'],
             'membership': membership, 'profile_preserved': True, 'launch_policy_preserved': True,
             'os_permissions_changed': False, 'applied': False}
    return value, p, record, previous


@contextmanager
def archive_view(archive, c):
    require(gate.hash_file(archive) == c['archive_sha256'], 'Fixed candidate archive digest differs')
    with guard.verified_archive(archive, policies(c)[1], identity_expectations=c['source']) as pair:
        up.fresh.verified_product(pair[0], archive, c['source'])
        yield pair


def plan(archive, home):
    c = contract()
    with archive_view(archive, c) as (candidate, signed):
        value, _, _, _ = make_plan(home, c, candidate, signed)
    return dict(value, plan_sha256=digest(value))


@contextmanager
def locked(p):
    path = p['state'].parent / '.desktop-install.lock'
    fd = os.open(path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'r+') as held:
        up.private(path)
        require(os.fstat(held.fileno()).st_ino == path.lstat().st_ino, 'Installer lock changed')
        fcntl.flock(held, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield


def write_new(path, raw):
    require(len(raw) <= up.LIMIT, 'Migration metadata exceeds bound')
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'wb') as out:
        out.write(raw); out.flush(); os.fsync(out.fileno())
    up.fsync(path.parent)


def checkpoint(p, data, phase, expected):
    require(phase in PHASES, 'Unknown signer migration phase')
    data['phase'] = phase
    return up.atomic_record(p['journal'], data, expected)


def runtime_record(before):
    # NativeControl.start uses only GUI presence and these public group digests.
    require(before.get('loaded') is True and before.get('background'),
            'Transition requires the original registered background to be running')
    network = (before['background'].get('network_diagnostics') or {}).get('network') or {}
    groups = network.get('relay_groups') or {}
    return {'loaded': True, 'gui': {'pid': before['gui']['pid']} if before.get('gui') else None,
            'background': {'network_diagnostics': {'network': {'relay_groups':
                {k: groups.get(k) for k in ('control_sha256', 'discovery_sha256')}}}}}


def migration_record(record, c):
    result = dict(record)
    previous = dict(result); previous.pop('history', None)
    require(isinstance(result.get('history', []), list), 'Unknown installation history')
    result.update(history=[*result.get('history', []), previous], source=c['source'],
                  archive_sha256=c['archive_sha256'], phase='upgrade_completed',
                  signed_application_verified=True, service_started=True,
                  launch_agent_preserved_sha256=c['current']['plist'], signer_transition=KIND)
    require(len(encoded(result)) < up.LIMIT - 4096, 'Installation history full; nothing discarded')
    return result


def apply(archive, home, expected_plan):
    c = contract(); gate.sha(expected_plan)
    with archive_view(archive, c) as (candidate, signed):
        value, p, _, _ = make_plan(home, c, candidate, signed)
        require(digest(value) == expected_plan, 'Reviewed migration plan drift')
        with locked(p):
            value2, p, record, previous = make_plan(home, c, candidate, signed)
            require(value2 == value, 'Migration changed under lock')
            next_record = migration_record(record, c)
            control = up.NativeControl(p)
            before = control.before(c['current']['executable'])
            before_record = runtime_record(before)
            data = {'schema': 1, 'kind': KIND, 'plan': value, 'plan_sha256': expected_plan,
                    'before': before_record, 'new_install': next_record,
                    'new_install_sha256': gate.digest(encoded(next_record))}
            data['phase'] = 'prepared'
            write_new(p['journal'], encoded(data)); journal_sha = gate.hash_file(p['journal'])
            p['rollback'].mkdir(mode=0o700); up.fsync(p['rollback'].parent)
            for name, path in (('install.before.json', p['state'] / 'install.json'),
                               ('upgrade.before.json', p['state'] / 'upgrade.json'),
                               ('launch-agent.before.plist', p['plist'])):
                write_new(p['rollback'] / name, path.read_bytes())
            staged = p['rollback'] / 'Weavelo.app'
            shutil.copytree(candidate, staged, symlinks=False)
            require(up.tree_digest(staged) == value['new_tree'], 'Staged candidate changed')
            guard.verify_app(staged, policies(c)[1])
            verify_backups(p, c)
            journal_sha = checkpoint(p, data, 'staged', journal_sha)
            _, _, _, _, membership = source_context(home, c)
            require(membership == value['membership'], 'Membership changed before normal quit')
            journal_sha = checkpoint(p, data, 'stopping', journal_sha)
            control.stop(before)
            journal_sha = checkpoint(p, data, 'stopped', journal_sha)
            with up.idle_locks(p['profile']):
                _, _, _, _, membership = source_context(home, c)
                require(membership == value['membership'] and gate.hash_file(archive) == c['archive_sha256'],
                        'Membership or archive changed before exchange')
                journal_sha = checkpoint(p, data, 'exchanging', journal_sha)
                up.exchange(staged, p['app'])
                journal_sha = checkpoint(p, data, 'exchanged', journal_sha)
                verify_tuple(p, c, value, 'new')
            journal_sha = checkpoint(p, data, 'activating', journal_sha)
            runtime = control.start(before, c['new_executable_sha256'])
            require(up.membership_metadata(p['profile']) == value['membership'], 'Membership changed during activation')
            verify_tuple(p, c, value, 'new')
            data['runtime'] = runtime
            journal_sha = checkpoint(p, data, 'activated', journal_sha)
            journal_sha = checkpoint(p, data, 'committing', journal_sha)
            up.atomic_record(p['state'] / 'install.json', next_record, c['current']['install'])
            checkpoint(p, data, 'completed', journal_sha)
            return dict(data, applied=True, rollback_retained=True)


def verify_backups(p, c):
    up.private(p['rollback'], True)
    require({v.name for v in p['rollback'].iterdir()} ==
            {'Weavelo.app', 'install.before.json', 'upgrade.before.json', 'launch-agent.before.plist'},
            'Unexpected rollback contents; preserve for inspection')
    for name, key in (('install.before.json', 'install'), ('upgrade.before.json', 'upgrade'),
                      ('launch-agent.before.plist', 'plist')):
        path = p['rollback'] / name
        up.private(path)
        require(gate.hash_file(path) == c['current'][key], 'Original rollback metadata changed')


def verify_unchanged(p, c):
    up.private(p['plist'])
    require(gate.hash_file(p['plist']) == c['current']['plist'], 'Launch policy drift; no restore attempted')
    up.validate_policy(plistlib.loads(p['plist'].read_bytes()), p)
    _, sha = up.read_record(p['state'] / 'upgrade.json')
    require(sha == c['current']['upgrade'], 'Normal upgrade state changed')


def verify_tuple(p, c, value, side):
    verify_backups(p, c); verify_unchanged(p, c)
    old, new = policies(c)
    active = old if side == 'old' else new
    other = new if side == 'old' else old
    require(up.tree_digest(p['app']) == value[side + '_tree'], 'Active bundle tuple changed')
    guard.verify_app(p['app'], active)
    staged = p['rollback'] / 'Weavelo.app'
    opposite = 'new' if side == 'old' else 'old'
    require(up.tree_digest(staged) == value[opposite + '_tree'], 'Rollback bundle tuple changed')
    guard.verify_app(staged, other)


def status(home):
    c = contract(); p = layout(home, c)
    if not p['journal'].exists() and not p['journal'].is_symlink():
        return {'kind': KIND, 'phase': 'not_started', 'applied': False,
                'rollback_path_exists': p['rollback'].exists() or p['rollback'].is_symlink()}
    data, sha = up.read_record(p['journal'])
    validate_transaction(data, p, c)
    return {'transaction': data, 'transaction_sha256': sha, 'observed': observe(p, c, data),
            'forward_resume_supported': False}


def observe(p, c, data):
    result = {}
    value = data['plan']
    for key, path in (('active', p['app']), ('rollback', p['rollback'] / 'Weavelo.app')):
        try:
            actual = up.tree_digest(path)
            side = 'old' if actual == value['old_tree'] else 'new' if actual == value['new_tree'] else 'unrecognized'
            if side != 'unrecognized': guard.verify_app(path, policies(c)[0 if side == 'old' else 1])
            result[key] = {'tree_sha256': actual, 'side': side, 'signature_verified': side != 'unrecognized'}
        except (OSError, guard.ReleaseRejected, gate.IdentityRejected):
            result[key] = {'side': 'unavailable_or_invalid', 'signature_verified': False}
    try:
        _, actual = up.read_record(p['state'] / 'install.json')
        result['registry'] = {'sha256': actual, 'side': 'old' if actual == c['current']['install'] else
                              'new' if actual == data['new_install_sha256'] else 'unrecognized'}
    except (OSError, ValueError, guard.ReleaseRejected):
        result['registry'] = {'side': 'unavailable_or_invalid'}
    return result


def validate_transaction(data, p, c):
    require(data.get('schema') == 1 and data.get('kind') == KIND and data.get('phase') in PHASES,
            'Unknown migration transaction')
    value = data['plan']
    require(digest(value) == data['plan_sha256'] and value.get('contract_sha256') == digest(c)
            and value.get('current') == c['current'] and value.get('source') == c['source']
            and value.get('archive_sha256') == c['archive_sha256']
            and value.get('old_tree') == c['current']['tree']
            and value.get('new_tree') == c['new_tree_sha256']
            and value.get('old_requirement') == policies(c)[0].requirement
            and value.get('new_requirement') == policies(c)[1].requirement,
            'Original migration plan binding changed')
    require(value['paths'] == {k: str(p[k]) for k in ('app', 'profile', 'plist', 'state', 'rollback', 'journal')},
            'Migration paths changed')
    gate.sha(value['new_tree'])
    require(gate.digest(encoded(data['new_install'])) == data['new_install_sha256'], 'New registry binding changed')
    require(data['new_install'].get('source') == c['source']
            and data['new_install'].get('signer_transition') == KIND, 'Unknown new installation record')


def restore_registry(path, raw, expected):
    # Exact original formatting is part of the pinned original registry digest.
    up.private(path)
    require(gate.hash_file(path) == expected, 'Registry changed before exact restore')
    temporary = path.with_name(path.name + '.signer-restore.pending')
    write_new(temporary, raw)
    require(gate.hash_file(path) == expected, 'Registry changed during exact restore')
    os.replace(temporary, path); up.fsync(path.parent)


def rollback(home, expected_transaction):
    c = contract(); p = layout(home, c); gate.sha(expected_transaction)
    with locked(p):
        data, sha = up.read_record(p['journal'])
        require(sha == expected_transaction, 'Original transaction version conflict')
        validate_transaction(data, p, c)
        require(data['phase'] != 'rolled_back', 'Rollback already completed; inspect, never replay')
        value = data['plan']; verify_unchanged(p, c)
        active_tree = up.tree_digest(p['app'])
        require(active_tree in (value['old_tree'], value['new_tree']), 'Unknown active bundle; no process stopped')
        side = 'old' if active_tree == value['old_tree'] else 'new'
        # Partial pre-stage failures are preserved for inspection, never guessed or cleaned.
        verify_tuple(p, c, value, side)
        original, _ = up.read_record(p['rollback'] / 'install.before.json')
        require(migration_record(original, c) == data['new_install'], 'Registry reconstruction differs')
        _, registry_sha = up.read_record(p['state'] / 'install.json')
        require(registry_sha in (c['current']['install'], data['new_install_sha256']),
                'User installation record changed; no restore attempted')
        control = up.NativeControl(p)
        expected_exe = c['current']['executable'] if side == 'old' else c['new_executable_sha256']
        current = control.before(expected_exe)
        # After completion preserve the currently running group's metadata/state;
        # an interrupted transition restores the original known running shape.
        restore = data.get('rollback_restore')
        if restore is None:
            restore = runtime_record(current) if data['phase'] == 'completed' else data['before']
        membership = up.membership_metadata(p['profile'])
        data['rollback_restore'] = restore
        sha = checkpoint(p, data, 'rollback_stopping', sha)
        control.stop(current)
        sha = checkpoint(p, data, 'rollback_stopped', sha)
        with up.idle_locks(p['profile']):
            verify_tuple(p, c, value, side)
            require(up.membership_metadata(p['profile']) == membership, 'Membership changed before rollback')
            _, fresh_sha = up.read_record(p['state'] / 'install.json')
            require(fresh_sha == registry_sha, 'Registry changed before rollback exchange')
            if side == 'new':
                sha = checkpoint(p, data, 'rollback_exchanging', sha)
                up.exchange(p['rollback'] / 'Weavelo.app', p['app'])
            sha = checkpoint(p, data, 'rollback_exchanged', sha)
            verify_tuple(p, c, value, 'old')
        sha = checkpoint(p, data, 'rollback_registry', sha)
        if registry_sha != c['current']['install']:
            # Restore exact original bytes, not a reserialized approximation.
            raw = (p['rollback'] / 'install.before.json').read_bytes()
            require(gate.digest(raw) == c['current']['install'], 'Original registry bytes changed')
            restore_registry(p['state'] / 'install.json', raw, registry_sha)
        sha = checkpoint(p, data, 'rollback_activating', sha)
        data['rollback_runtime'] = control.start(restore, c['current']['executable'])
        require(up.membership_metadata(p['profile']) == membership, 'Membership changed during rollback activation')
        verify_tuple(p, c, value, 'old')
        checkpoint(p, data, 'rolled_back', sha)
        return dict(data, rolled_back=True, profile_preserved=True, failed_candidate_retained=True)


def cli():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('archive', nargs='?', type=Path)
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument('--apply', action='store_true')
    modes.add_argument('--rollback', action='store_true')
    modes.add_argument('--status', action='store_true')
    parser.add_argument('--expected-plan-sha256')
    parser.add_argument('--expected-transaction-sha256')
    args = parser.parse_args()
    try:
        require(sys.platform == 'darwin', 'Native macOS required')
        if args.status:
            require(not args.archive and not args.expected_plan_sha256 and not args.expected_transaction_sha256,
                    'Status does not accept mutation pins')
            result = status(Path.home())
        elif args.rollback:
            require(not args.archive and not args.expected_plan_sha256 and args.expected_transaction_sha256,
                    'Rollback requires only the exact observed transaction digest')
            result = rollback(Path.home(), args.expected_transaction_sha256)
        else:
            require(args.archive and not args.expected_transaction_sha256, 'Fixed archive required')
            if args.apply:
                require(args.expected_plan_sha256, 'Reviewed plan digest required')
                result = apply(args.archive, Path.home(), args.expected_plan_sha256)
            else:
                require(not args.expected_plan_sha256, 'Plan does not accept mutation pins')
                result = plan(args.archive, Path.home())
        print(json.dumps(result, indent=2))
    except (guard.ReleaseRejected, gate.IdentityRejected, OSError, ValueError, KeyError, TypeError,
            subprocess.SubprocessError) as error:
        mutating = args.apply or args.rollback
        print(json.dumps({'applied': None if mutating else False,
                          'outcome': 'not_confirmed' if mutating else 'not_applied',
                          'detail': str(error) if isinstance(error, guard.ReleaseRejected) else 'Required evidence unavailable',
                          'next_action': 'Read original --status and actual bundle tuple; never forward replay'}), file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(cli())
