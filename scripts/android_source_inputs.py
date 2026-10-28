"""Fixed Android source inventory for the primary checkout and existing producer.

This records source inputs, not a hermetic toolchain or a signing authorization.
Desktop WIP is outside the Android dependency closure; Android inputs must match
the selected commit, and the producer scripts must match the observed HEAD.
"""
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import tomllib

SCHEMA = 'weavelo.android-source-inputs.v1'
PRODUCT_ROOTS = ('android', 'protocol', 'remote_core', 'remote_client_bridge')
PRODUCT_FILES = ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml',
                 'deploy/desktop/product-identity.json')
PRODUCER_FILES = ('scripts/android_source_inputs.py', 'scripts/build_android_native.sh',
                  'scripts/build_android_release.sh', 'scripts/release_artifact_lifecycle.py',
                  'scripts/verify_release_apk.py', 'scripts/lifecycle_preload_gate.py',
                  'scripts/cmake/android-arm64.cmake', 'scripts/cmake/android-x86_64.cmake')
GENERATED = ('android/.gradle', 'android/.kotlin', 'android/build', 'android/app/build',
             'android/app/src/main/jniLibs')
LOCAL_CONFIGURATION = 'android/local.properties'


def require(ok, reason):
    if not ok:
        raise ValueError('Android source inputs: ' + reason)


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()


def digest(value):
    return hashlib.sha256(value).hexdigest()


def git(root, *args):
    result = subprocess.run(['git', '--no-optional-locks', '-C', str(root), *args],
                            capture_output=True, timeout=30)
    require(result.returncode == 0, 'cannot read the fixed Git source')
    return result.stdout


def tree(root, commit):
    require(bool(re.fullmatch('[0-9a-f]{40}', commit)), 'a full commit is required')
    records = {}
    for row in git(root, 'ls-tree', '-r', '-z', commit).split(b'\0'):
        if not row:
            continue
        fields, name = row.split(b'\t', 1)
        mode, kind, object_id = fields.decode().split()
        records[name.decode()] = (mode, kind, object_id)
    return records


def product_path(name):
    return (name in PRODUCT_FILES or name.startswith('.cargo/')
            or Path(name).name == 'Cargo.toml'
            or any(name.startswith(prefix + '/') for prefix in PRODUCT_ROOTS))


def read_file(root, name):
    path = root / name
    require(not name.lower().endswith(('.pem', '.key', '.p12', '.pfx'))
            and path.name not in ('mesh.conf', 'mesh.secret'), 'private material is not a source input')
    require(not any(p.is_symlink() for p in (path, *path.parents)), 'linked input: ' + name)
    before = path.stat()
    require(stat.S_ISREG(before.st_mode) and before.st_nlink == 1
            and before.st_size <= 32 * 1024**2, 'input is not a bounded regular file: ' + name)
    fd = os.open(path, os.O_RDONLY | getattr(os, 'O_NOFOLLOW', 0))
    def stable(info):
        return (info.st_dev, info.st_ino, info.st_mode, info.st_size, info.st_mtime_ns, info.st_ctime_ns)
    with os.fdopen(fd, 'rb') as incoming:
        require(stable(os.fstat(incoming.fileno())) == stable(before), 'input changed before read: ' + name)
        data = incoming.read()
        require(stable(os.fstat(incoming.fileno())) == stable(before), 'input changed during read: ' + name)
    require(stable(path.stat()) == stable(before), 'input changed after read: ' + name)
    return data, '100755' if before.st_mode & 0o111 else '100644'


def records(root, baseline, names):
    result = []
    for name in sorted(names):
        mode, kind, object_id = baseline[name]
        require(kind == 'blob' and mode in ('100644', '100755'), 'linked/submodule input: ' + name)
        data, actual_mode = read_file(root, name)
        require(actual_mode == mode and data == git(root, 'cat-file', 'blob', object_id),
                'working bytes differ from the fixed commit: ' + name)
        result.append({'path': name, 'mode': mode, 'sha256': digest(data)})
    return result


def check_inventory(root, selected):
    # Inspect ignored files too: an ignored Kotlin/Rust source must not silently
    # enter compilation. Only the fixed generated output directories are pruned.
    observed = set()
    for prefix in (*PRODUCT_ROOTS, '.cargo'):
        start = root / prefix
        if not start.exists():
            continue
        require(not start.is_symlink(), 'linked source directory: ' + prefix)
        for directory, children, files in os.walk(start, followlinks=False):
            base = Path(directory)
            for child in list(children):
                path = (base / child).relative_to(root).as_posix()
                if path in GENERATED:
                    children.remove(child)
                else:
                    require(not (base / child).is_symlink(), 'linked source directory: ' + path)
            observed.update((base / name).relative_to(root).as_posix() for name in files)
    unexpected = observed - selected - {LOCAL_CONFIGURATION}
    require(not unexpected, 'uncommitted input files: ' + ', '.join(sorted(unexpected)[:8]))


def check_local_dependencies(root):
    supported = {'protocol', 'remote_core', 'remote_client_bridge'}
    for package in supported:
        manifest = tomllib.loads((root / package / 'Cargo.toml').read_text())
        sections = [manifest, *manifest.get('target', {}).values()]
        for section in sections:
            for group in ('dependencies', 'build-dependencies', 'dev-dependencies'):
                for dependency in section.get(group, {}).values():
                    if not isinstance(dependency, dict):
                        continue
                    require(not dependency.get('workspace'), 'workspace dependency requires inventory review')
                    if 'path' in dependency:
                        target = (root / package / dependency['path']).resolve()
                        require(target.parent == root and target.name in supported,
                                'local dependency requires inventory review: ' + package)


def snapshot(root, product_commit=None):
    root = Path(root).absolute()
    require(root == root.resolve() and root.is_dir(), 'canonical primary source root required')
    git_root = Path(git(root, 'rev-parse', '--show-toplevel').decode().strip()).resolve()
    require(str(git_root).casefold() == str(root).casefold() and git_root.samefile(root),
            'expected repository root')
    require(git(root, 'branch', '--show-current').decode().strip() == 'main', 'primary main required')
    head = git(root, 'rev-parse', 'HEAD').decode().strip()
    product_commit = product_commit or head
    baseline, producer = tree(root, product_commit), tree(root, head)
    # A source pin cannot select another branch's product while using main tools.
    git(root, 'merge-base', '--is-ancestor', product_commit, head)
    selected = {name for name in baseline if product_path(name)}
    require(selected == {name for name in producer if product_path(name)}, 'input inventory changed since source pin')
    require(set(PRODUCT_FILES) <= selected, 'required product inputs missing')
    require(set(PRODUCER_FILES) <= producer.keys(), 'producer scripts must be committed')
    require(not any(name == p or name.startswith(p + '/') for name in selected for p in GENERATED),
            'generated outputs cannot be committed source inputs')
    require(len(selected) <= 1024, 'input inventory exceeds bound')
    check_inventory(root, selected)
    product = records(root, baseline, selected)
    tools = records(root, producer, PRODUCER_FILES)
    check_local_dependencies(root)
    local = root / LOCAL_CONFIGURATION
    local_record = ({'path': LOCAL_CONFIGURATION, 'sha256': digest(read_file(root, LOCAL_CONFIGURATION)[0])}
                    if local.exists() else None)
    require(git(root, 'rev-parse', 'HEAD').decode().strip() == head, 'HEAD changed during input capture')
    result = {'schema': SCHEMA, 'product_commit': product_commit, 'producer_commit': head,
              'product_files': product, 'producer_files': tools, 'local_configuration': local_record}
    return dict(result, manifest_sha256=digest(canonical(result)))


def validate_record(value):
    require(isinstance(value, dict) and value.get('schema') == SCHEMA, 'input manifest missing')
    unsigned = {key: item for key, item in value.items() if key != 'manifest_sha256'}
    require(value.get('manifest_sha256') == digest(canonical(unsigned)), 'input manifest digest differs')
    require(all(re.fullmatch('[0-9a-f]{40}', value.get(key, ''))
                for key in ('product_commit', 'producer_commit')), 'input manifest commits invalid')
    return value


def verify(root, expected):
    validate_record(expected)
    require(snapshot(root, expected['product_commit']) == expected, 'inputs changed during the build')
    return expected
