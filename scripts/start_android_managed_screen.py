#!/usr/bin/env python3
"""Start Weavelo's installed capture helper through an already authorized ADB device.

This does not pair ADB, grant permissions, change security settings, or install APKs.
After attachment, video and input stay in the phone's existing Weavelo session.
"""
import argparse
import json
import os
from pathlib import Path
import shlex
import subprocess
import time

PACKAGE = 'fun.hackerlife.weavelo'
PROVIDER = 'content://' + PACKAGE + '.managed'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--serial', required=True)
    parser.add_argument('--adb', default=str(Path(os.environ.get('ANDROID_HOME', Path.home() / 'Library/Android/sdk')) / 'platform-tools/adb'))
    args = parser.parse_args()
    adb = [args.adb, '-s', args.serial]

    def run(*command, timeout=15):
        return subprocess.check_output(adb + list(command), text=True, stderr=subprocess.STDOUT, timeout=timeout).strip()

    if run('get-state') != 'device' or run('shell', 'id', '-u') != '2000':
        raise RuntimeError('Use the existing device-authorized, non-root ADB shell')
    if int(run('shell', 'getprop', 'ro.build.version.sdk')) < 34:
        raise RuntimeError('Automatic capture currently requires Android 14 or later')
    if run('shell', 'am', 'get-current-user') != '0':
        raise RuntimeError('This helper serves Weavelo in the phone owner profile; the active profile is different')
    paths = run('shell', 'pm', 'path', PACKAGE).splitlines()
    apk = next((line.removeprefix('package:') for line in paths if line.endswith('/base.apk')), None)
    if not apk or not apk.startswith('/data/app/'):
        raise RuntimeError('Installed Weavelo base APK was not found')

    def status():
        result = run('shell', 'content', 'call', '--uri', PROVIDER, '--method', 'status')
        if 'status=unavailable' in result:
            return None
        if 'status=' not in result:
            raise RuntimeError('Install the current Weavelo with its capture provider before activation')
        value, _ = json.JSONDecoder().raw_decode(result.split('status=', 1)[1])
        if value.get('api') != 2:
            raise RuntimeError('Unsupported capture helper API')
        return value

    existing = status()
    if existing is not None:
        print(json.dumps({'reused': True, 'helper': existing}))
        return
    # Opening the user-visible app permits its normal foreground-service start.
    run('shell', 'am', 'start', '-W', '-n', PACKAGE + '/.MainActivity')
    log = '/data/local/tmp/weavelo-managed-screen.log'
    # Keep the guard in the foreground. Backgrounding an AND-list leaves an
    # intermediate shell holding ADB's output pipe until the helper exits.
    command = ('test ! -L ' + shlex.quote(log) + ' || exit 1\nCLASSPATH=' + shlex.quote(apk)
               + ' nohup setsid app_process /system/bin ' + PACKAGE + '.managed.ManagedScreenMain'
               + ' >' + shlex.quote(log) + ' 2>&1 </dev/null &')
    run('shell', command)
    end = time.monotonic() + 15
    while time.monotonic() < end:
        current = status()
        if current is not None:
            print(json.dumps({'reused': False, 'helper': current}))
            return
        time.sleep(.25)
    detail = run('shell', 'tail', '-n', '12', log)
    raise RuntimeError('Weavelo capture helper did not attach: ' + detail)


if __name__ == '__main__':
    main()
