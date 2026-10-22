#!/usr/bin/env python3
"""Reconstruct one exact build-local crate; never edit Cargo registry caches.

Only recipe + replacement are publication inputs, never this script's output.
Preparation and --verify fail on existing tampered output rather than repairing
it silently. Delete .work/dependency-remediation explicitly to reconstruct anew.
"""
from pathlib import Path, PurePosixPath
import argparse
import hashlib
import io
import json
import tarfile
import urllib.request

RECIPE_SHA256 = '8fc9437995504ca2e0fcb50cf210279bafdfe0a61d3de28552ecd6983cfb84fd'


def digest(data):
    return hashlib.sha256(data).hexdigest()


def contained(root, path):
    if not path.resolve().is_relative_to(root.resolve()):
        raise RuntimeError('reconstructed path escapes project')
    for parent in [path, *path.parents]:
        if parent == root:
            break
        if parent.is_symlink():
            raise RuntimeError('reconstructed symlink rejected')
    return path


def prepare(root, archive_path=None, verify_only=False):
    root = Path(root).resolve()
    inputs = root / 'scripts/remediation'
    recipe_bytes = (inputs / 'recipe.json').read_bytes()
    if digest(recipe_bytes) != RECIPE_SHA256:
        raise RuntimeError('recipe SHA256 mismatch')
    recipe = json.loads(recipe_bytes)
    replacement = (inputs / recipe['replacement_file']).read_bytes()
    if digest(replacement) != recipe['replacement_sha256']:
        raise RuntimeError('replacement SHA256 mismatch')
    cache = contained(root, root / '.work/dependency-remediation/archive.crate')
    output = contained(root, root / recipe['output'])
    if archive_path is not None:
        data = Path(archive_path).read_bytes()
    elif cache.is_file():
        data = cache.read_bytes()
    elif verify_only:
        raise RuntimeError('Missing reconstruction archive; run preparation first')
    else:
        with urllib.request.urlopen(recipe['registry_archive_url'], timeout=60) as response:
            if response.geturl() != recipe['registry_archive_url']:
                raise RuntimeError('Unexpected reconstruction archive redirect')
            data = response.read(16 * 1024 * 1024 + 1)
    if len(data) > 16 * 1024 * 1024 or digest(data) != recipe['registry_archive_sha256']:
        raise RuntimeError('archive SHA256 mismatch')
    expected = {}
    prefix = recipe['name'] + '-' + recipe['version']
    with tarfile.open(fileobj=io.BytesIO(data), mode='r:gz') as archive:
        for member in archive.getmembers():
            path = PurePosixPath(member.name)
            if path.is_absolute() or '..' in path.parts or not path.parts or path.parts[0] != prefix:
                raise RuntimeError('Unsafe reconstruction archive path')
            if member.isdir():
                continue
            if not member.isfile() or len(path.parts) < 2:
                raise RuntimeError('Non-regular reconstruction archive member')
            relative = PurePosixPath(*path.parts[1:]).as_posix()
            if relative in expected:
                raise RuntimeError('Duplicate reconstruction archive member')
            stream = archive.extractfile(member)
            if stream is None:
                raise RuntimeError('Missing reconstruction member')
            expected[relative] = stream.read()
    if digest(expected[recipe['file']]) != recipe['original_sha256']:
        raise RuntimeError('original file SHA256 mismatch')
    vcs = json.loads(expected['.cargo_vcs_info.json'])
    if vcs['git']['sha1'] != recipe['commit'] or vcs['path_in_vcs'] != recipe['path_in_vcs']:
        raise RuntimeError('Archive VCS provenance mismatch')
    expected[recipe['file']] = replacement
    if output.exists():
        actual = {}
        for path in output.rglob('*'):
            contained(root, path)
            if path.is_file():
                actual[path.relative_to(output).as_posix()] = path.read_bytes()
            elif not path.is_dir():
                raise RuntimeError('Non-regular reconstructed output')
        if actual != expected:
            raise RuntimeError('reconstructed source tree mismatch')
    elif verify_only:
        raise RuntimeError('Missing reconstructed source tree')
    else:
        output.mkdir(parents=True)
        for relative, content in expected.items():
            target = contained(root, output / relative)
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(content)
    if not verify_only:
        cache.parent.mkdir(parents=True, exist_ok=True)
        if cache.exists() and digest(cache.read_bytes()) != recipe['registry_archive_sha256']:
            raise RuntimeError('cached archive SHA256 mismatch')
        cache.write_bytes(data)
    records = {name: digest(content) for name, content in sorted(expected.items())}
    result = {'schema': 1, 'variant': recipe['variant'], 'name': recipe['name'], 'version': recipe['version'],
              'output': recipe['output'], 'archive_sha256': recipe['registry_archive_sha256'],
              'recipe_sha256': RECIPE_SHA256, 'replacement_sha256': recipe['replacement_sha256'],
              'files': len(records), 'tree_sha256': digest(json.dumps(records, sort_keys=True, separators=(',', ':')).encode())}
    if not verify_only:
        (cache.parent / 'provenance.json').write_text(json.dumps(result, indent=2) + '\n', encoding='utf-8')
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--verify', action='store_true', help='verify only; no network or writes')
    parser.add_argument('--archive', type=Path, help='optional exact locally retained archive')
    args = parser.parse_args()
    print(json.dumps(prepare(Path(__file__).resolve().parents[1], archive_path=args.archive, verify_only=args.verify), indent=2))
