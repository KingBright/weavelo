#!/usr/bin/env python3
"""Pinned upgrade of an existing independent Windows Desktop installation.

Consumes the original observed producer and replaces the installed files in place.
No membership import, policy/ACL repair, legacy-client stop or automatic rollback.
An existing transaction is observed with --status, never replayed.
"""
import argparse
import ctypes
from ctypes import wintypes as W
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

import build_identity_gate as gate
import install_windows_independent as install
import install_native_first_identity as original
import windows_private_profile as private
from windows_process_lifecycle import stop_verified

require = gate.require


def process_metadata(pid):
    k = ctypes.WinDLL('kernel32', use_last_error=True)
    def bind(name, args, result):
        f = getattr(k, name); f.argtypes = args; f.restype = result
        return f
    opened = bind('OpenProcess', [W.DWORD, W.BOOL, W.DWORD], W.HANDLE)
    close = bind('CloseHandle', [W.HANDLE], W.BOOL)
    image = bind('QueryFullProcessImageNameW', [W.HANDLE, W.DWORD, W.LPWSTR, ctypes.POINTER(W.DWORD)], W.BOOL)
    times = bind('GetProcessTimes', [W.HANDLE] + [ctypes.POINTER(W.FILETIME)] * 4, W.BOOL)
    exited = bind('GetExitCodeProcess', [W.HANDLE, ctypes.POINTER(W.DWORD)], W.BOOL)
    handle = opened(0x1000, False, pid)
    require(handle, 'Ordinary process metadata unavailable')
    try:
        data = [W.FILETIME() for _ in range(4)]
        buffer = ctypes.create_unicode_buffer(32768); size = W.DWORD(len(buffer)); code = W.DWORD()
        require(times(handle, *map(ctypes.byref, data)) and image(handle, 0, buffer, ctypes.byref(size))
                and exited(handle, ctypes.byref(code)) and code.value == 259, 'Original owner is not running')
        return {'pid': pid, 'path': os.path.normcase(buffer.value),
                'created': (data[0].dwHighDateTime << 32) | data[0].dwLowDateTime}
    finally:
        close(handle)


def membership_metadata(profile):
    """Only file identity/size/times: never read authentication contents."""
    result = {}
    for name in ('mesh.conf', 'mesh.secret'):
        path = profile/name
        private.ordinary_path(path)
        if path.exists():
            value = path.stat()
            result[name] = [value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns]
    return result


def transaction_paths(home):
    p = install.paths(home)
    return dict(p, journal=p['state']/'upgrade.json',
                rollback=p['app'].parent/'.desktop-upgrade.rollback',
                pending=p['app'].parent/'.desktop-upgrade.pending')


def current(home, old_binary, old_identity, pid, created):
    p = transaction_paths(home)
    record = install.status(home)
    require(record.get('applied') is True, 'Completed independent install required')
    require(record['binary_sha256'] == old_binary and record['identity_sha256'] == old_identity,
            'Current installation pins changed')
    require(original.inventory(p['app'], gate) == record['payload'], 'Installed payload changed')
    executable = p['app']/'weavelo.exe'
    info = json.loads(subprocess.check_output([str(executable), '--product-info-json'], text=True, timeout=15))
    expected = gate.validate_identity(info['build_identity'], record['runtime_commit'], old_identity)
    require(gate.product_info(executable, expected, old_binary) == info, 'Installed executable changed')
    process = process_metadata(pid)
    require(process == {'pid': pid, 'path': os.path.normcase(str(executable)), 'created': created},
            'Original process path or creation time changed')
    return {'registry': record, 'registry_sha256': gate.hash_file(p['state']/'install.json'),
            'identity': expected, 'process': process, 'membership': membership_metadata(p['profile'])}


def plan(home, bound, old_binary, old_identity, pid, created):
    p = transaction_paths(home)
    for name in ('journal', 'pending'):
        private.ordinary_path(p[name])
    require(not p['pending'].exists(), 'Unfinished upgrade staging requires repair')
    c = current(home, old_binary, old_identity, pid, created)
    prior = None
    if p['journal'].exists():
        previous = gate.read_json(p['journal'])
        previous_paths = previous.get('plan', {}).get('paths', {})
        require(previous.get('schema') == 1 and previous.get('phase') == 'completed'
                and previous.get('kind') == 'independent-weavelo-windows-desktop-upgrade'
                and set(previous_paths) == set(p)
                and all(os.path.normcase(previous_paths[k]) == os.path.normcase(str(v)) for k, v in p.items())
                and previous.get('plan', {}).get('candidate', {}).get('binary_sha256') == old_binary,
                'Unfinished or unrelated upgrade transaction requires repair')
        prior = gate.hash_file(p['journal'])
    require(bound['binary_sha256'] != old_binary, 'No product change; no upgrade')
    require(install.revalidate(bound) == bound['files'], 'Candidate producer changed')
    value = {'schema': 1, 'kind': 'independent-weavelo-windows-desktop-upgrade',
             'paths': {k: str(v) for k, v in p.items()}, 'current': c,
             'candidate': {'commit': bound['commit'], 'binary_sha256': bound['binary_sha256'],
                           'identity_sha256': bound['identity_sha256'], 'archive_sha256': bound['archive_sha256'],
                           'receipt_sha256': bound['receipt_sha256'], 'payload': bound['files']},
             'legacy_clients_modified': False, 'security_policy_changed': False,
             'update_mode': 'in-place', 'previous_upgrade_sha256': prior}
    return value, gate.digest(original.canonical(value))


def commit_registry(path, expected_sha, previous, candidate, runtime):
    require(gate.hash_file(path) == expected_sha, 'Install registry changed; no overwrite')
    value = dict(previous)
    history = value.get('upgrade_history', [])
    require(isinstance(history, list), 'Unknown history field; preserve without overwrite')
    history = history + [{k: previous[k] for k in
                          ('runtime_commit', 'identity_sha256', 'binary_sha256', 'archive_sha256')}]
    value.update(runtime_commit=candidate['commit'], identity_sha256=candidate['identity_sha256'],
                 binary_sha256=candidate['binary_sha256'], archive_sha256=candidate['archive_sha256'],
                 build_receipt_sha256=candidate['receipt_sha256'], payload=candidate['payload'],
                 phase='committed_running', service_or_GUI_started=True, upgrade_history=history,
                 last_upgrade={'pid': runtime['pid'], 'membership_imported': False})
    original.atomic_state(path, value)
    require(json.loads(path.read_text()) == value, 'Registry independent readback differs')
    return value


def runtime_env(profile):
    return dict(os.environ, WEAVELO_DEVICE_GROUP_DIR=str(profile),
                WEAVELO_HOST_BIND_ADDR='0.0.0.0:39273', WEAVELO_DISCOVERY_PORT='38119',
                WEAVELO_RELAY='1', WEAVELO_RELAY_SERVER_ADDR='wss://weavelo.hackerlife.fun:8443/v1/relay',
                WEAVELO_P2P='0')


class NativeOwners:
    """The two authenticated Desktop owners, verified again before stopping."""
    def __init__(self, executable, profile):
        self.exe, self.profile = executable, profile

    def query(self, flag, required=True):
        r = subprocess.run([str(self.exe), flag], env=runtime_env(self.profile),
                           capture_output=True, text=True, timeout=15)
        if r.returncode and not required:
            return None
        require(r.returncode == 0, 'Desktop owner query failed: ' + flag)
        return json.loads(r.stdout)

    def pids(self):
        literal = str(self.exe).replace("'", "''")
        command = "$p='" + literal + "';@(Get-Process -Name weavelo -ErrorAction SilentlyContinue | Where-Object {$_.Path -eq $p} | ForEach-Object {$_.Id}) | ConvertTo-Json -Compress"
        r = subprocess.run(['powershell.exe', '-NoProfile', '-Command', command],
                           capture_output=True, text=True, timeout=15)
        require(r.returncode == 0, 'Cannot enumerate Desktop owners')
        values = json.loads(r.stdout) if r.stdout.strip() else []
        values = values if isinstance(values, list) else [values]
        require(all(type(v) is int and v > 1 for v in values), 'Invalid Desktop owner IDs')
        return set(values)

    def observe(self, binary, primary_pid):
        background = self.query('--background-status-json')
        require(background.get('role') == 'background-service'
                and background.get('pid') == primary_pid
                and background.get('executable_sha256') == binary, 'Background owner changed')
        gui = self.query('--gui-status-json', False)
        gui_pid = None
        if gui and gui.get('ok') is True:
            require(gui.get('executable_sha256') == binary
                    and gui.get('renderer') == 'restored-original-gpui', 'GUI owner changed')
            # Older Desktop releases expose the PID in the generation only.
            gui_pid = gui.get('pid') or int(gui['generation'].split('-', 1)[0])
            require(type(gui_pid) is int and gui_pid > 1 and gui_pid != primary_pid, 'Invalid GUI owner')
        ids = {primary_pid} | ({gui_pid} if gui_pid else set())
        require(self.pids() == ids, 'Unidentified Desktop process; repair before updating')
        processes = {str(pid): process_metadata(pid) for pid in ids}
        require(all(v['path'] == os.path.normcase(str(self.exe)) for v in processes.values()), 'Desktop process path changed')
        return {'background_pid': primary_pid, 'frontend_pid': gui_pid, 'processes': processes}

    def stop(self, before, binary, expected):
        require(self.pids() == {int(pid) for pid in before['processes']}, 'Desktop owners changed before stop')
        class Owner:
            def native(_self, pid, executable, digest, identity):
                require(process_metadata(pid) == before['processes'][str(pid)], 'Desktop process was replaced')
                return gate.verify_running_windows(pid, executable, digest, identity)
        for pid in (before['frontend_pid'], before['background_pid']):
            if pid:
                stop_verified(Owner(), pid, self.exe, binary, expected)
        require(not self.pids(), 'Desktop owner remains; files were not replaced')

    def start_gui(self, binary):
        child = subprocess.Popen([str(self.exe)], env=runtime_env(self.profile), stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            creationflags=subprocess.CREATE_NEW_PROCESS_GROUP | subprocess.DETACHED_PROCESS)
        deadline = time.monotonic() + 40
        while time.monotonic() < deadline:
            status = self.query('--gui-status-json', False)
            if status and status.get('ok') is True and status.get('executable_sha256') == binary:
                pid = status.get('pid') or int(status['generation'].split('-', 1)[0])
                require(pid == child.pid and status.get('renderer') == 'restored-original-gpui', 'Updated GUI identity differs')
                return pid
            time.sleep(.5)
        raise RuntimeError('Updated GUI readiness unconfirmed; repair the current update')


def start(executable, profile):
    return subprocess.Popen([str(executable), '--background-service'], env=runtime_env(profile),
        stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        creationflags=subprocess.CREATE_NEW_PROCESS_GROUP | subprocess.DETACHED_PROCESS)


def ready(executable, profile, pid, candidate):
    env = runtime_env(profile)
    for _ in range(12):
        call = subprocess.run([str(executable), '--background-status-json'], env=env,
                              capture_output=True, text=True, timeout=6)
        if call.returncode == 0:
            value = json.loads(call.stdout)
            require(value['pid'] == pid and value['executable_sha256'] == candidate['binary_sha256']
                    and value['network_diagnostics']['build_identity']['source']['commit'] == candidate['commit'],
                    'Actual runtime differs from the verified package')
            pair = subprocess.run([str(executable), '--pairing-status-json'], env=env,
                                  capture_output=True, text=True, timeout=6)
            require(pair.returncode == 0, 'Installed pairing status still unavailable')
            return {'pid': pid, 'status': value, 'pairing': json.loads(pair.stdout)}
        time.sleep(.5)
    raise RuntimeError('New owner readiness unconfirmed; inspect original transaction')


def apply(home, bound, old_binary, old_identity, pid, created, expected_plan,
          security=None, stopper=None, starter=None, reader=None):
    require(sys.platform == 'win32', 'Native Windows apply required')
    reviewed, digest = plan(home, bound, old_binary, old_identity, pid, created)
    require(digest == gate.sha(expected_plan), 'Upgrade plan changed')
    p = transaction_paths(home); native = security or private.NativeSecurity()
    state = {'schema': 1, 'kind': reviewed['kind'], 'plan_sha256': digest, 'plan': reviewed, 'phase': 'verified'}
    prior = reviewed['previous_upgrade_sha256']
    require(prior is None or gate.hash_file(p['journal']) == prior, 'Previous upgrade changed')
    original.atomic_state(p['journal'], state, initial=prior is None)
    def checkpoint(phase, **extra):
        state.update(phase=phase, **extra); original.atomic_state(p['journal'], state)
    native.create_private_directory(p['pending'])
    shutil.copytree(bound['candidate'], p['pending'], dirs_exist_ok=True)
    require(original.inventory(p['pending'], gate) == bound['files'], 'Staged payload changed')
    checkpoint('staged')
    now = current(home, old_binary, old_identity, pid, created)
    require(now == reviewed['current'] and install.revalidate(bound) == bound['files'],
            'Installation or producer changed before stop')
    class Owner:
        def native(self, actual_pid, executable, binary, expected):
            require(process_metadata(actual_pid) == now['process'], 'Original owner was replaced')
            return gate.verify_running_windows(actual_pid, executable, binary, expected)
    owners = None if stopper else NativeOwners(p['app']/'weavelo.exe', p['profile'])
    before = owners.observe(old_binary, pid) if owners else None
    if owners:
        owners.stop(before, old_binary, now['identity'])
    else:
        stopper(Owner(), pid, p['app']/'weavelo.exe', old_binary, now['identity'])
    checkpoint('stopped')
    require(original.inventory(p['pending'], gate) == bound['files']
            and original.inventory(p['app'], gate) == now['registry']['payload']
            and gate.hash_file(p['state']/'install.json') == now['registry_sha256'],
            'Payload or registry changed during stop; inspect original transaction')
    checkpoint('updating_files', owners_before=before, rollback_policy='none_new')
    for relative in bound['files']:
        target = p['app']/relative
        target.parent.mkdir(parents=True, exist_ok=True)
        private.ordinary_path(target)
        os.replace(p['pending']/relative, target)
    new_names = {name.casefold() for name in bound['files']}
    for relative, digest in now['registry']['payload'].items():
        if relative.casefold() not in new_names:
            target = p['app']/relative
            require(gate.hash_file(target) == digest, 'Obsolete installed file changed')
            target.unlink()
    require(original.inventory(p['app'], gate) == bound['files'], 'Installed payload differs; repair in place')
    require(not any(member.is_file() or member.is_symlink() for member in p['pending'].rglob('*')), 'Unknown staging content retained')
    shutil.rmtree(p['pending'])
    checkpoint('installed_not_started')
    child = (starter or start)(p['app']/'weavelo.exe', p['profile'])
    checkpoint('started_unconfirmed', new_pid=child.pid)
    runtime = (reader or ready)(p['app']/'weavelo.exe', p['profile'], child.pid, reviewed['candidate'])
    receipt = gate.read_json(bound['receipt'], bound['receipt_sha256'])
    gate.verify_running_windows(child.pid, p['app']/'weavelo.exe', bound['binary_sha256'], receipt['build_identity'])
    if owners and before['frontend_pid']:
        runtime['frontend_pid'] = owners.start_gui(bound['binary_sha256'])
        gate.verify_running_windows(runtime['frontend_pid'], p['app']/'weavelo.exe', bound['binary_sha256'], receipt['build_identity'])
    require(membership_metadata(p['profile']) == now['membership'], 'Membership files changed; inspect original')
    commit_registry(p['state']/'install.json', now['registry_sha256'], now['registry'], reviewed['candidate'], runtime)
    checkpoint('completed', actual=runtime, rollback_preserved=False, prior_rollback_untouched=True, membership_contents_read=False)
    return state


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('build-receipt-sha256', 'expected-archive-sha256', 'expected-commit',
                 'expected-identity-sha256', 'expected-current-binary-sha256',
                 'expected-current-identity-sha256', 'expected-plan-sha256'):
        parser.add_argument('--'+name)
    parser.add_argument('--current-pid', type=int)
    parser.add_argument('--expected-current-created', type=int)
    parser.add_argument('--apply', action='store_true')
    parser.add_argument('--status', action='store_true')
    args = parser.parse_args(); home = Path.home()
    if args.status:
        require(not args.apply, 'Status cannot apply')
        path = transaction_paths(home)['journal']
        private.ordinary_path(path)
        value = gate.read_json(path) if path.exists() else {'phase': 'not-started'}
    else:
        require(all((args.build_receipt_sha256, args.expected_archive_sha256, args.expected_commit,
                     args.expected_identity_sha256, args.expected_current_binary_sha256,
                     args.expected_current_identity_sha256, args.current_pid, args.expected_current_created)),
                'Exact producer and current-owner pins required')
        bound = install.reviewed(args)
        values = (home, bound, gate.sha(args.expected_current_binary_sha256),
                  gate.sha(args.expected_current_identity_sha256), args.current_pid, args.expected_current_created)
        if args.apply:
            require(args.expected_plan_sha256, 'Apply requires reviewed plan SHA256')
            value = apply(*values, args.expected_plan_sha256)
        else:
            reviewed, digest = plan(*values); value = {'plan': reviewed, 'plan_sha256': digest}
    print(json.dumps(value, indent=2))


if __name__ == '__main__':
    main()
