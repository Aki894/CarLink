"""Persistent per-device HiCar identity and fixed six-digit PIN, kept outside Git."""
import argparse
from dataclasses import dataclass, field
import json
import os
from pathlib import Path
import re
import secrets
import stat
import sys
import tempfile

DEFAULT_PATH = '/var/lib/carlink/hicar/identity.json'


@dataclass(frozen=True)
class Identity:
    device_id: str = field(repr=False)
    pin: str = field(repr=False)
    name: str = 'CarLink HiCar'

    def validate(self):
        if not isinstance(self.device_id, str) or not re.fullmatch(r'[0-9a-f]{64}', self.device_id):
            raise ValueError('invalid local device identity')
        if not isinstance(self.pin, str) or not re.fullmatch(r'[0-9]{6}', self.pin):
            raise ValueError('PIN must be exactly six ASCII digits')
        if not isinstance(self.name, str) or not 1 <= len(self.name) <= 64:
            raise ValueError('invalid local device name')
        return self

    def encoded(self):
        self.validate()
        return json.dumps(dict(schema=1, device_id=self.device_id, pin=self.pin,
                               name=self.name), ensure_ascii=False).encode('utf-8')


def load(path):
    fd = os.open(path, os.O_RDONLY | getattr(os, 'O_NOFOLLOW', 0))
    with os.fdopen(fd, 'rb') as handle:
        info = os.fstat(handle.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_size > 4096:
            raise ValueError('invalid identity file')
        if os.name == 'posix' and info.st_mode & 0o077:
            raise ValueError('identity file must be private (mode 0600)')
        record = json.load(handle)
    if not isinstance(record, dict) or record.get('schema') != 1:
        raise ValueError('unsupported identity schema')
    return Identity(record.get('device_id'), record.get('pin'), record.get('name')).validate()


def _write(path, identity, create):
    path = Path(path)
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix='.identity-', dir=path.parent)
    try:
        with os.fdopen(fd, 'wb') as handle:
            handle.write(identity.encoded())
            handle.flush()
            os.fsync(handle.fileno())
        if create:
            os.link(temporary, path)  # Atomic no-overwrite publication.
        else:
            os.replace(temporary, path)
        if os.name == 'posix':
            directory = os.open(path.parent, os.O_RDONLY | getattr(os, 'O_DIRECTORY', 0))
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
    finally:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass


def load_or_create(path=DEFAULT_PATH):
    try:
        return load(path)
    except FileNotFoundError:
        identity = Identity(secrets.token_hex(32), f'{secrets.randbelow(1000000):06d}')
        try:
            _write(path, identity, create=True)
            return identity
        except FileExistsError:
            return load(path)


def set_pin(path, pin):
    old = load(path)
    changed = Identity(old.device_id, pin, old.name).validate()
    _write(path, changed, create=False)
    return changed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['init', 'show-pin', 'set-pin'])
    parser.add_argument('--path', default=DEFAULT_PATH)
    args = parser.parse_args()
    try:
        if args.command == 'init':
            load_or_create(args.path)
            print('HiCar identity initialized; fixed PIN retained in private configuration.')
        elif args.command == 'show-pin':
            print(load(args.path).pin)  # Explicit operator request only.
        else:
            set_pin(args.path, sys.stdin.readline().rstrip('\r\n'))
            print('Fixed HiCar PIN updated; device identity retained.')
    except (ValueError, OSError) as error:
        parser.exit(1, f'{type(error).__name__}: private identity operation failed\n')


if __name__ == '__main__':
    main()
