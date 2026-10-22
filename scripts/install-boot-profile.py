"""Install/verify/revert a bounded boot profile in an OFFLINE mounted image.

No target command is executed. Backups contain only the explicit managed files.
"""
import argparse
import base64
import json
import os
from pathlib import Path, PurePosixPath
import stat

BACKUP = 'etc/carlink/boot-profile-backup.json'
PROFILE = Path(__file__).resolve().parents[1] / 'firmware/boot-profile'


def image_root(root):
    root = Path(root).absolute()
    if root.is_symlink():
        raise ValueError('Image root must not be a symbolic link')
    root = root.resolve(strict=True)
    if root == Path(root.anchor) or not root.is_dir():
        raise ValueError('Refusing execution host root or non-directory')
    if not (root / 'etc').is_dir():
        raise ValueError('Expected a mounted Linux image root with etc/')
    return root


def safe_path(root, name):
    relative = PurePosixPath(name)
    if (relative.is_absolute() or '..' in relative.parts or '\\' in name
            or ':' in name or str(relative) != name):
        raise ValueError('Unsafe managed path: ' + name)
    path = root / name
    for parent in path.parents:
        if parent == root:
            break
        if parent.is_symlink() or (parent.exists() and not parent.is_dir()):
            raise ValueError('Linked or non-directory destination parent: ' + name)
    return path


def snapshot(path):
    if path.is_symlink():
        return {'kind': 'link', 'target': os.readlink(path)}
    if not path.exists():
        return {'kind': 'absent'}
    if not path.is_file():
        raise ValueError('Refusing to replace non-file: ' + str(path))
    if path.stat().st_size > 1024 * 1024:
        raise ValueError('Unexpectedly large configuration file')
    return {'kind': 'file', 'data': base64.b64encode(path.read_bytes()).decode(),
            'mode': stat.S_IMODE(path.stat().st_mode) if os.name != 'nt' else 0o644}


def write(path, state):
    if path.is_symlink() or path.exists():
        if not path.is_symlink() and not path.is_file():
            raise ValueError('Refusing to remove non-file: ' + str(path))
        path.unlink()
    if state['kind'] == 'absent':
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    if state['kind'] == 'link':
        path.symlink_to(state['target'])
    elif state['kind'] == 'file':
        path.write_bytes(base64.b64decode(state['data'], validate=True))
        path.chmod(state['mode'])
    else:
        raise ValueError('Unknown backup entry type')


def desired(profile=PROFILE):
    policy = json.loads((profile / 'policy.json').read_text(encoding='utf-8'))
    if set(policy['masks']) & set(policy['preserved']):
        raise ValueError('Boot policy masks a preserved service')
    files = {}
    for path in sorted((profile / 'rootfs').rglob('*')):
        if path.is_symlink():
            raise ValueError('Linked boot profile source')
        if path.is_file():
            name = path.relative_to(profile / 'rootfs').as_posix()
            files[name] = {'kind': 'file', 'mode': 0o644,
                           'data': base64.b64encode(path.read_bytes()).decode()}
    for unit in policy['masks']:
        if '/' in unit or unit.startswith('.'):
            raise ValueError('Unsafe mask unit name')
        files['etc/systemd/system/' + unit] = {'kind': 'link', 'target': '/dev/null'}
    for name, target in policy['links'].items():
        files[name] = {'kind': 'link', 'target': target}
    for name in policy.get('remove', []):
        files[name] = {'kind': 'absent'}
    return files


def validate_state(state):
    if state.get('kind') == 'absent' and set(state) == {'kind'}:
        return
    if state.get('kind') == 'link' and set(state) == {'kind', 'target'}:
        if isinstance(state['target'], str):
            return
    if state.get('kind') == 'file' and set(state) == {'kind', 'data', 'mode'}:
        if isinstance(state['mode'], int) and 0 <= state['mode'] <= 0o7777:
            if len(base64.b64decode(state['data'], validate=True)) <= 1024 * 1024:
                return
    raise ValueError('Malformed boot profile backup state')


def operate(root, action='install', profile=PROFILE):
    root = image_root(root)
    planned = desired(profile)
    paths = {name: safe_path(root, name) for name in planned}
    backup_path = safe_path(root, BACKUP)
    if backup_path.is_symlink():
        raise ValueError('Linked backup manifest')
    record = None
    if backup_path.exists():
        record = json.loads(backup_path.read_text(encoding='utf-8'))
        if record.get('version') != 1 or set(record.get('entries', {})) != set(planned):
            raise ValueError('Backup manifest differs from managed path allowlist')
        for name, entry in record['entries'].items():
            validate_state(entry['before'])
            validate_state(entry['after'])
            if snapshot(paths[name]) != entry['after']:
                raise ValueError('Managed file changed since installation: ' + name)
    if action == 'revert':
        if record is None:
            raise ValueError('No boot profile backup to restore')
        for name, entry in record['entries'].items():
            write(paths[name], entry['before'])
        backup_path.unlink()
        return {'action': action, 'files': len(paths)}
    if action == 'verify':
        if record is None:
            raise ValueError('Boot profile backup missing')
        for name, state in planned.items():
            if snapshot(paths[name]) != state:
                raise ValueError('Boot profile content mismatch: ' + name)
        return {'action': action, 'files': len(paths)}
    if record is not None:
        if all(entry['after'] == planned[name] for name, entry in record['entries'].items()):
            return {'action': 'unchanged', 'files': len(paths)}
        raise ValueError('Revert existing profile before installing a different version')
    entries = {name: {'before': snapshot(path), 'after': planned[name]}
               for name, path in paths.items()}
    record = {'version': 1, 'entries': entries}
    backup_path.parent.mkdir(parents=True, exist_ok=True)
    # Write recovery data before changing any configuration.
    backup_path.write_text(json.dumps(record, indent=2) + '\n', encoding='utf-8')
    backup_path.chmod(0o600)
    changed = []
    try:
        for name, state in planned.items():
            changed.append(name)
            write(paths[name], state)
    except Exception:
        for name in reversed(changed):
            write(paths[name], entries[name]['before'])
        backup_path.unlink()
        raise
    return {'action': action, 'files': len(paths)}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', required=True, type=Path)
    parser.add_argument('--profile', type=Path, default=PROFILE)
    group = parser.add_mutually_exclusive_group()
    group.add_argument('--revert', action='store_true')
    group.add_argument('--verify', action='store_true')
    args = parser.parse_args()
    print(json.dumps(operate(args.root, 'revert' if args.revert else 'verify'
                             if args.verify else 'install', args.profile)))
