"""Validate CI application artifacts, then install into a mounted image root."""
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import struct
import sys


def checksums(directory):
    declared = set()
    for line in (directory / 'SHA256SUMS').read_text().splitlines():
        digest, name = line.split('  ', 1)
        relative = PurePosixPath(name)
        if (len(digest) != 64 or any(c not in '0123456789abcdef' for c in digest)
                or relative.is_absolute() or '..' in relative.parts or '\\' in name
                or ':' in name or name in declared):
            raise ValueError('Unsafe or duplicate checksum entry')
        path = directory / name
        if path.is_symlink() or not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != digest:
            raise ValueError('Artifact checksum mismatch: ' + name)
        declared.add(name)
    for path in directory.rglob('*'):
        if path.is_symlink():
            raise ValueError('Linked artifact payload')
        if path.is_file():
            relative = path.relative_to(directory).as_posix()
            if path.suffix.lower() in ('.pk8', '.p7b', '.key', '.p12', '.pem'):
                raise ValueError('Private runtime material in artifact')
            if relative != 'SHA256SUMS' and relative not in declared:
                # The app artifact additionally carries source/toolchain metadata.
                if relative not in ('sources.lock.json', 'elf-header.txt'):
                    raise ValueError('Unhashed artifact payload: ' + relative)
    return declared


def armhf(path):
    data = path.read_bytes()[:52]
    if len(data) < 52 or data[:6] != b'\x7fELF\x01\x01':
        raise ValueError('Expected little-endian ELF32')
    if struct.unpack_from('<H', data, 18)[0] != 40 or not struct.unpack_from('<I', data, 36)[0] & 0x400:
        raise ValueError('Expected ARM hard-float ELF')


def install(project, app, receiver, rootfs):
    rootfs = rootfs.resolve()
    if rootfs == Path(rootfs.anchor):
        raise ValueError('Refusing to install into the execution host root')
    manifest = json.loads((rootfs / 'etc/carlink-build.json').read_text())
    lock = json.loads((project / 'sources.lock.json').read_text())
    source = json.loads((receiver / 'SOURCE.json').read_text())
    if source['project_commit'] != os.environ['APP_COMMIT']:
        raise ValueError('Receiver artifact source run mismatch')
    for key in ('rust', 'target', 'relay', 'stream'):
        if source[key] != lock['carplay'][key]:
            raise ValueError('Receiver source lock mismatch: ' + key)
    app_lock = json.loads((app / 'sources.lock.json').read_text())
    if app_lock['application'] != lock['application']:
        raise ValueError('Application toolchain lock mismatch')
    actual_patches = {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                      for p in (project / 'receiver/patches').glob('*.patch')}
    recorded_patches = {PurePosixPath(p['file']).name: p['sha256'] for p in source['patches']}
    if actual_patches != recorded_patches:
        raise ValueError('Receiver patches do not match current source')
    app_files = checksums(app)
    receiver_files = checksums(receiver)
    if 'carlink' not in app_files:
        raise ValueError('Application binary missing from checksum manifest')
    required = ['bin/openxplay-relay', 'bin/oxp-relay', 'probe.py', 'forward.py', 'supervisor.py', 'run-carplay.sh', 'run-carlink.sh', 'bin/carlink-audio', 'share/source/audio/SOURCE.json', 'share/source/audio/ffmpeg.tar.gz', 'share/licenses/audio/COPYING.LGPLv2.1', 'SOURCE.json',
                'share/web/receiver/index.html', 'share/admin/index.html', 'share/source/relay.tar.gz',
                'share/source/stream.tar.gz', 'share/licenses/relay/LICENSE',
                'share/licenses/relay/LICENSE-GPL-3.0', 'share/licenses/relay/NOTICE',
                'share/licenses/stream/LICENSE']
    for name in required:
        if name not in receiver_files or not (receiver / name).stat().st_size:
            raise ValueError('Missing receiver payload: ' + name)
    audio_source = json.loads((receiver / 'share/source/audio/SOURCE.json').read_text())
    for key, value in lock['audio_transcoder'].items():
        if audio_source[key] != value:
            raise ValueError('Audio transcoder source lock mismatch: ' + key)
    for path in (app / 'carlink', receiver / 'bin/openxplay-relay', receiver / 'bin/oxp-relay', receiver / 'bin/carlink-audio'):
        armhf(path)
    destination = (rootfs / 'usr/local/lib/carlink/carplay').resolve()
    if not destination.is_relative_to(rootfs):
        raise ValueError('Receiver destination escapes image root')
    # A clean replacement avoids retaining old, unhashed runtime files.
    if destination.exists():
        shutil.rmtree(destination)
    shutil.copytree(receiver, destination)
    for name in ('bin/openxplay-relay', 'bin/oxp-relay', 'bin/carlink-audio', 'probe.py', 'forward.py', 'supervisor.py', 'run-carplay.sh', 'run-carlink.sh'):
        (destination / name).chmod(0o755)
    bindir = rootfs / 'usr/local/bin'
    bindir.mkdir(parents=True, exist_ok=True)
    for src, name in ((app / 'carlink', 'carlink'), (receiver / 'run-carplay.sh', 'carlink-carplay'), (receiver / 'run-carlink.sh', 'carlink-run')):
        shutil.copyfile(src, bindir / name)
        (bindir / name).chmod(0o755)
    app_destination=(rootfs/'usr/local/lib/carlink/app').resolve()
    if not app_destination.is_relative_to(rootfs):
        raise ValueError('Application source destination escapes image root')
    if app_destination.exists():
        shutil.rmtree(app_destination)
    if (app/'share').is_dir():
        shutil.copytree(app,app_destination)
    for name in ('var/lib/carlink/carplay', 'var/lib/carlink/carplay/mfi'):
        directory = rootfs / name
        directory.mkdir(parents=True, exist_ok=True)
        directory.chmod(0o700)
    manifest.update(project_commit=os.environ['REFRESH_COMMIT'], sources=lock,
                    app_source_commit=os.environ['APP_COMMIT'], app_run_id=os.environ['APP_RUN_ID'],
                    app_sha256=hashlib.sha256((app / 'carlink').read_bytes()).hexdigest(),
                    app_notices=(app/'share').is_dir(),
                    carplay_receiver=source, stage='CarLife MD and separate ARMv7 CarPlay receiver')
    (rootfs / 'etc/carlink-build.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print('CI application and CarPlay runtime installed after checksum, ARM ABI and source validation')


if __name__ == '__main__':
    if len(sys.argv) != 4:
        raise SystemExit('Usage: install-projection-runtime.py APP_DIR RECEIVER_DIR MOUNTED_ROOT')
    install(Path(__file__).resolve().parents[1], *(Path(p).resolve() for p in sys.argv[1:]))
