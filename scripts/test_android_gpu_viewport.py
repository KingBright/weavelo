#!/usr/bin/env python3
"""Compile current native GPU adapters and test synthetic pixels via ADB.

Does not install an APK, capture a real display, alter permissions, or publish.
The fixture uses the actual product OES renderer and hardware HEVC adapter.
"""
import argparse
import os
import re
from pathlib import Path
import subprocess
import sys
import zipfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--serial', required=True, help='Explicit authorized USB ADB serial')
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[1]
    work = repo / '.rust-storage/validation-logs/android-gpu-viewport'
    work.mkdir(parents=True, exist_ok=True)
    sdk = Path(os.environ.get('ANDROID_HOME', Path.home() / 'Library/Android/sdk'))
    java_home = os.environ.get('JAVA_HOME')
    if not java_home and sys.platform == 'darwin':
        java_home = subprocess.check_output(['/usr/libexec/java_home', '-v', '17'], text=True).strip()
    java = Path(java_home) / 'bin' if java_home else Path('/usr/bin')
    android = sdk / 'platforms/android-35/android.jar'
    env = {**os.environ, 'ANDROID_HOME': str(sdk)}
    if java_home:
        env['JAVA_HOME'] = java_home
    log = work / 'probe.log'

    def run(command, cwd=repo, timeout=180):
        try:
            result = subprocess.run([str(part) for part in command], cwd=cwd, env=env,
                                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=timeout)
        except subprocess.TimeoutExpired as error:
            partial = error.stdout or b''
            with log.open('a') as output:
                output.write(partial.decode(errors='replace') if isinstance(partial, bytes) else partial)
            raise
        with log.open('a') as output:
            output.write(result.stdout)
        if result.returncode:
            raise RuntimeError(result.stdout[-8000:])
        return result.stdout

    log.write_text('Synthetic native GPU fixture; no OS screen capture or installation.\n')
    run(['./gradlew', '--offline', '--no-configuration-cache', '-Pkotlin.incremental=false',
         '-Pkotlin.compiler.execution.strategy=in-process', ':app:compileDebugKotlin'], repo / 'android')
    classes = repo / 'android/app/build/tmp/kotlin-classes/debug'
    stdlibs = sorted((Path.home() / '.gradle/caches/modules-2/files-2.1/org.jetbrains.kotlin/kotlin-stdlib').glob('*/*/*.jar'))
    if not stdlibs:
        raise RuntimeError('Existing Kotlin standard library not found; run normal Android compilation first')
    kotlin = re.search(r'id\("org\.jetbrains\.kotlin\.android"\) version "([^"]+)"',
                       (repo / 'android/settings.gradle.kts').read_text())
    if not kotlin:
        raise RuntimeError('Current project Kotlin version not found')
    stdlib = next((path for path in stdlibs if '/' + kotlin[1] + '/' in str(path)), None)
    if stdlib is None:
        raise RuntimeError('Current project Kotlin standard library is not in the existing cache')
    compiled = work / 'classes'
    compiled.mkdir(exist_ok=True)
    run([java / 'javac', '--release', '17', '-classpath', os.pathsep.join(map(str, [android, classes, stdlib])),
         '-d', compiled, repo / 'scripts/android_gpu_viewport_probe.java'])
    inputs = work / 'probe.jar'
    with zipfile.ZipFile(inputs, 'w') as archive:
        for name in ('GpuVideoSurface', 'DisplayOwner', 'SurfaceHevcEncoder', 'VideoDimensions', 'VideoFrameGeometry'):
            for source in (classes / 'fun/hackerlife/weavelo').glob(name + '*.class'):
                archive.write(source, source.relative_to(classes))
        for source in compiled.rglob('*.class'):
            archive.write(source, source.relative_to(compiled))
    dex = work / 'dex'
    dex.mkdir(exist_ok=True)
    d8s = sorted((sdk / 'build-tools').glob('*/lib/d8.jar'), key=lambda path: tuple(int(n) for n in path.parent.parent.name.split('.')))
    if not d8s:
        raise RuntimeError('Existing Android build-tools D8 not found')
    run([java / 'java', '-cp', d8s[-1], 'com.android.tools.r8.D8',
         '--min-api', '26', '--lib', android, '--output', dex, inputs, stdlib])
    adb = [sdk / 'platform-tools/adb', '-s', args.serial]
    model = run(adb + ['shell', 'getprop', 'ro.product.model']).strip()
    remote = '/data/local/tmp/weavelo-gpu-viewport.dex'
    try:
        run(adb + ['push', dex / 'classes.dex', remote])
        result = run(adb + ['shell', 'CLASSPATH=' + remote, 'app_process', '/system/bin',
                            'fun.hackerlife.weavelo.AndroidGpuViewportProbe'], timeout=30)
        passed = [line for line in result.splitlines() if line.startswith('WEAVELO_GPU_VIEWPORT_PASS ')]
        if len(passed) != 1:
            raise RuntimeError('Native fixture returned without its result: ' + result[-8000:])
        print('DEVICE', args.serial, model)
        print(passed[0])
        print('LOG', log)
    finally:
        run(adb + ['shell', 'rm', '-f', remote])


if __name__ == '__main__':
    main()
