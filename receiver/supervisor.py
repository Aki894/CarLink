#!/usr/bin/env python3
"""Supervise the two projection endpoints; systemd owns restart/backoff."""
import json
import errno
import os
from pathlib import Path
import signal
import socket
import stat
import subprocess
import time
import sys
import tempfile


def bootstrap_config(destination, example):
    """Publish the shipped configuration once, preserving existing local state."""
    destination, example = Path(destination), Path(example)
    if os.path.lexists(destination):
        return False
    data = example.read_bytes()
    config = json.loads(data)
    if len(data) > 65536 or config.get('schema') != 1:
        raise ValueError('Invalid shipped manager configuration')
    destination.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix='.manager-', dir=destination.parent)
    try:
        with os.fdopen(fd, 'wb') as stream:
            os.chmod(temporary, 0o600)
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        try:
            # link publishes only if absent, unlike rename which overwrites.
            os.link(temporary, destination)
        except FileExistsError:
            return False
        return True
    finally:
        os.unlink(temporary)


def clear_stale_socket(name):
    """Remove only a same-uid, unchanged endpoint with no listening owner."""
    try:
        before = os.lstat(name)
    except FileNotFoundError:
        return False
    if not stat.S_ISSOCK(before.st_mode) or before.st_uid != os.geteuid():
        raise RuntimeError('Projection endpoint is not an owned socket')
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as probe:
        probe.settimeout(.25)
        try:
            probe.connect(str(name))
        except OSError as error:
            if error.errno != errno.ECONNREFUSED:
                raise RuntimeError('Projection endpoint owner is unverified') from None
        else:
            raise RuntimeError('Projection endpoint has an active owner')
    after = os.lstat(name)
    if (after.st_dev, after.st_ino, after.st_uid, after.st_mode) != (
            before.st_dev, before.st_ino, before.st_uid, before.st_mode):
        raise RuntimeError('Projection endpoint changed during inspection')
    os.unlink(name)
    return True


def main():
    children = []
    stopping = False

    def stop(*_):
        nonlocal stopping
        stopping = True

    for sig in (signal.SIGTERM, signal.SIGINT):
        signal.signal(sig, stop)
    os.umask(0o077)
    live = os.environ.get('CARLINK_LIVE_SOCKET', '/run/carlink/live.sock')
    for path in ('/run/carlink', '/run/carlink-carplay'):
        Path(path).mkdir(mode=0o700, parents=True, exist_ok=True)
    endpoints = (live, '/run/carlink-carplay/media.sock')
    # Only one legacy owner may inspect or reclaim these private endpoints.
    import fcntl
    ownership = os.open('/run/carlink/supervisor.lock', os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK, 0o600)
    lock_stat = os.fstat(ownership)
    if not stat.S_ISREG(lock_stat.st_mode) or lock_stat.st_uid != os.geteuid():
        raise RuntimeError('Projection lock is not an owned regular file')
    fcntl.flock(ownership, fcntl.LOCK_EX | fcntl.LOCK_NB)
    for name in endpoints:
        clear_stale_socket(name)
    commands = [
        ['/usr/local/bin/carlink', 'carlife', 'usb', '--duration', '0', '--live-socket', live],
        ['/usr/local/bin/carlink-carplay', 'forward'],
    ]
    try:
        for command in commands:
            children.append(subprocess.Popen(command))
        print(json.dumps({'event': 'carlink_started', 'monotonic_s': round(time.monotonic(), 3)}), flush=True)
        while not stopping and all(child.poll() is None for child in children):
            time.sleep(.2)
    finally:
        for child in children:
            if child.poll() is None:
                child.terminate()
        deadline = time.monotonic() + 10
        for child in children:
            try:
                child.wait(timeout=max(.1, deadline - time.monotonic()))
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=2)
        for name in endpoints:
            try:
                clear_stale_socket(name)
            except (FileNotFoundError, RuntimeError):
                # A replacement/live owner must retain its endpoint.
                continue
        os.close(ownership)
    return 0 if stopping else 1


if __name__ == '__main__':
    if len(sys.argv) == 4 and sys.argv[1] == '--bootstrap':
        bootstrap_config(sys.argv[2], sys.argv[3])
        raise SystemExit(0)
    raise SystemExit(main())
