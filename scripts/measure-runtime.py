#!/usr/bin/env python3
"""Read-only Linux runtime benchmark. Never reads arguments or identity files."""
import argparse
import json
import os
from pathlib import Path
import time

GROUP = Path('/sys/fs/cgroup/system.slice/carlink.service')


def snapshot():
    counters = dict(line.split() for line in (GROUP / 'cpu.stat').read_text().splitlines())
    processes = []
    for pid in (GROUP / 'cgroup.procs').read_text().split():
        root = Path('/proc') / pid
        try:
            memory = {}
            for line in (root / 'smaps_rollup').read_text().splitlines():
                if ':' in line:
                    key, value = line.split(':', 1)
                    if key in ('Rss', 'Pss', 'Private_Dirty', 'Private_Clean'):
                        memory[key + '_kib'] = int(value.split()[0])
            processes.append(dict(pid=int(pid), name=Path(os.readlink(root / 'exe')).name, **memory))
        except (OSError, ValueError):
            continue  # A worker may finish while being sampled.
    return dict(monotonic=time.monotonic(), cpu_us=int(counters['usage_usec']),
                cgroup_memory_bytes=int((GROUP / 'memory.current').read_text()),
                pss_kib=sum(p.get('Pss_kib', 0) for p in processes), processes=processes)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--seconds', type=float, default=5)
    parser.add_argument('--label', default='runtime')
    args = parser.parse_args()
    if not 1 <= args.seconds <= 60:
        parser.error('--seconds must be between 1 and 60')
    if not GROUP.is_dir():
        parser.error('carlink.service cgroup v2 is unavailable')
    before = snapshot()
    time.sleep(args.seconds)
    after = snapshot()
    elapsed = after['monotonic'] - before['monotonic']
    print(json.dumps(dict(label=args.label, seconds=round(elapsed, 3),
                         cpu_percent_one_core=round((after['cpu_us'] - before['cpu_us']) / elapsed / 10000, 2),
                         pss_kib=after['pss_kib'], cgroup_memory_bytes=after['cgroup_memory_bytes'],
                         processes=after['processes'])))


if __name__ == '__main__':
    main()
