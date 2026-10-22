#!/usr/bin/env python3
"""Supervise the two projection endpoints; systemd owns restart/backoff."""
import json
import os
from pathlib import Path
import signal
import stat
import subprocess
import time


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
        # These endpoints belong to the terminated child processes in private dirs.
        for name in (live, '/run/carlink-carplay/media.sock'):
            path = Path(name)
            try:
                if stat.S_ISSOCK(path.lstat().st_mode):
                    path.unlink()
            except FileNotFoundError:
                pass
    return 0 if stopping else 1


if __name__ == '__main__':
    raise SystemExit(main())
