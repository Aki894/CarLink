"""Install/verify/revert a bounded boot profile in an OFFLINE mounted image.

No target command is executed. Backups contain only the explicit managed files.
"""
import argparse
import base64
import json
import os
from pathlib import Path, PurePosixPath
import stat
import tempfile

BACKUP = 'etc/carlink/boot-profile-backup.json'
PROFILE = Path(__file__).resolve().parents[1] / 'firmware/boot-profile'

# Explicit historical path sets keep a backup manifest from authorizing writes
# outside the files owned by a known profile version.
V1_PATHS = frozenset({
    'etc/systemd/journald.conf.d/80-carlink.conf',
    'etc/systemd/system/apt-daily-upgrade.timer.d/80-carlink.conf',
    'etc/systemd/system/apt-daily.timer.d/80-carlink.conf',
    'etc/systemd/system/carlink.service',
    'etc/systemd/system/carlink.service.d/80-journal.conf',
    'etc/systemd/system/NetworkManager-wait-online.service',
    'etc/systemd/system/serial-getty@ttyGS0.service',
    'etc/systemd/system/ModemManager.service',
    'etc/systemd/system/motd-news.service',
    'etc/systemd/system/motd-news.timer',
    'etc/systemd/system/man-db.timer',
    'etc/systemd/system/multi-user.target.wants/carlink.service',
    'etc/systemd/system/default.target',
    'etc/systemd/system/getty.target.wants/serial-getty@ttyGS0.service',
})
V2_ADDITIONS = frozenset({
    'etc/systemd/system/rsyslog.service',
    'etc/systemd/system/syslog.socket',
    'etc/systemd/system/getty@tty1.service',
    'etc/systemd/system/keyboard-setup.service',
    'etc/systemd/system/console-setup.service',
    'etc/systemd/system/logrotate.timer.d/80-carlink.conf',
})
VERSION_PATHS = {1: V1_PATHS, 2: V1_PATHS | V2_ADDITIONS}
LEGACY_V1_PATHS = V1_PATHS - {'etc/systemd/system/carlink.service.d/80-journal.conf'}


def known_paths(version, names):
    if type(version) is not int or version not in VERSION_PATHS:
        return False
    return names == VERSION_PATHS[version] or (version == 1 and names == LEGACY_V1_PATHS)


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
    if not isinstance(state, dict):
        raise ValueError('Malformed boot profile backup state')
    if state.get('kind') == 'absent' and set(state) == {'kind'}:
        return
    if state.get('kind') == 'link' and set(state) == {'kind', 'target'}:
        if (isinstance(state['target'], str) and '\0' not in state['target']
                and 0 < len(state['target']) <= 4096):
            return
    if state.get('kind') == 'file' and set(state) == {'kind', 'data', 'mode'}:
        if isinstance(state['mode'], int) and 0 <= state['mode'] <= 0o7777:
            if len(base64.b64decode(state['data'], validate=True)) <= 1024 * 1024:
                return
    raise ValueError('Malformed boot profile backup state')


def save_record(path, record):
    """Publish recovery data without truncating the previous manifest."""
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, name = tempfile.mkstemp(prefix='.boot-profile-', dir=path.parent)
    try:
        with os.fdopen(descriptor, 'w', encoding='utf-8') as stream:
            os.chmod(name, 0o600)
            stream.write(json.dumps(record, indent=2) + '\n')
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(name, path)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def operate(root, action='install', profile=PROFILE):
    root = image_root(root)
    planned = desired(profile)
    version = json.loads((profile / 'policy.json').read_text(encoding='utf-8'))['version']
    if not known_paths(version, set(planned)):
        raise ValueError('Unknown boot profile version or managed path allowlist')
    paths = {name: safe_path(root, name) for name in planned}
    backup_path = safe_path(root, BACKUP)
    if backup_path.is_symlink():
        raise ValueError('Linked backup manifest')
    record = None
    if backup_path.exists():
        record = json.loads(backup_path.read_text(encoding='utf-8'))
        if not isinstance(record, dict):
            raise ValueError('Malformed boot profile backup manifest')
        installed_version = record.get('profile_version', 1)
        if (record.get('version') != 1 or not isinstance(record.get('entries'), dict)
                or not known_paths(installed_version, set(record['entries']))):
            raise ValueError('Backup manifest differs from managed path allowlist')
        for name, entry in record['entries'].items():
            paths[name] = safe_path(root, name)
            if not isinstance(entry, dict) or set(entry) != {'before', 'after'}:
                raise ValueError('Malformed boot profile backup entry')
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
        return {'action': action, 'files': len(record['entries'])}
    if action == 'verify':
        if record is None:
            raise ValueError('Boot profile backup missing')
        if installed_version != version or set(record['entries']) != set(planned):
            raise ValueError('Boot profile upgrade required')
        for name, state in planned.items():
            if snapshot(paths[name]) != state:
                raise ValueError('Boot profile content mismatch: ' + name)
        return {'action': action, 'files': len(paths)}
    if record is not None:
        if installed_version > version:
            raise ValueError('Refusing boot profile downgrade; revert explicitly first')
        if (installed_version == version and set(record['entries']) == set(planned)
                and all(entry['after'] == planned[name] for name, entry in record['entries'].items())):
            return {'action': 'unchanged', 'files': len(paths)}
    previous = record
    transition = {name: snapshot(path) for name, path in paths.items()}
    entries = {name: {'before': (previous['entries'][name]['before']
                               if previous and name in previous['entries']
                               else transition[name]),
                      'after': (planned[name] if name in planned
                                else previous['entries'][name]['after'])}
               for name in paths}
    record = {'version': 1, 'profile_version': version, 'entries': entries}
    # Write recovery data before changing any configuration.
    save_record(backup_path, record)
    changed = []
    try:
        for name, state in planned.items():
            changed.append(name)
            write(paths[name], state)
    except Exception:
        for name in reversed(changed):
            write(paths[name], transition[name])
        if previous is None:
            backup_path.unlink()
        else:
            save_record(backup_path, previous)
        raise
    return {'action': 'upgrade' if previous else action, 'files': len(paths)}


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
