"""Formal image gates shared by mounted-image validation and clean-root fixtures."""
import hashlib
import importlib.util
import json
from pathlib import Path, PurePosixPath
import re
import struct
import sys


def load_installer():
    spec = importlib.util.spec_from_file_location('runtime_install', Path(__file__).with_name('install-projection-runtime.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def payload(root, relative):
    p = PurePosixPath(relative)
    if p.is_absolute() or '..' in p.parts or '\\' in relative or ':' in relative:
        raise ValueError('Unsafe production payload path')
    target = root / p
    if not target.resolve().is_relative_to(root.resolve()) or not target.is_file() or not target.stat().st_size:
        raise ValueError('Missing production payload: ' + relative)
    return target


def verify(root):
    root = Path(root).resolve()
    installer = load_installer()
    build = json.loads(payload(root, 'etc/carlink-build.json').read_text(encoding='utf-8'))
    if build.get('runtime_profile') != 'production':
        raise ValueError('Image is not a formal production runtime')
    example = json.loads(payload(root, 'usr/share/carlink/manager.example.json').read_text(encoding='utf-8'))
    if (example.get('schema') != 1 or not isinstance(example.get('hicar'), list)
            or 'hicar' not in example['hicar'] or 'run' not in example['hicar']
            or not all(example.get(name) for name in ('carlife', 'carplay', 'menu_library', 'menu_font'))):
        raise ValueError('Invalid production manager example')
    for relative in ('usr/local/bin/carlink', 'usr/local/bin/carlink-run', 'usr/local/bin/carlink-carplay'):
        payload(root, relative)
    expected = build.get('hicar_native_source')
    if not expected:
        raise ValueError('Production HiCar native source is missing')
    native = root / 'usr/local/lib/carlink/hicar-native'
    if not native.resolve().is_relative_to(root):
        raise ValueError('Production native path escapes image root')
    declared = installer.checksums(native)
    for relative in ('sources.json', 'libcarlink_hichain.so', 'lib/libcrypto.so.3', 'lib/libmbedcrypto.so.7', 'lib/libcjson.so.1'):
        if relative not in declared:raise ValueError('Missing production HiCar native payload: ' + relative)
        if relative != 'sources.json':installer.armhf(native / relative)
    if json.loads((native / 'sources.json').read_text(encoding='utf-8')) != expected:
        raise ValueError('Production HiCar native source differs')
    if hashlib.sha256((native / 'libcarlink_hichain.so').read_bytes()).hexdigest() != build.get('hicar_native_sha256'):
        raise ValueError('Production HiCar native checksum differs')
    driver = json.loads(payload(root, 'etc/carlink-aic-driver.json').read_text(encoding='utf-8'))
    checks = json.loads(payload(root, 'etc/carlink-aic-checksums.json').read_text(encoding='utf-8'))
    kernel = driver.get('kernel_release', '')
    if not re.fullmatch(r'[A-Za-z0-9._+-]+', kernel) or not re.fullmatch('[0-9a-f]{40}', driver.get('driver_commit', '')):
        raise ValueError('Invalid production AIC driver source')
    if driver.get('variant') not in ('main', 'mcu1'):
        raise ValueError('Invalid production AIC profile')
    modules = driver.get('modules', [])
    wanted = {f'modules/{kernel}/extra/aic8800/{name}.ko' for name in ('aic_load_fw', 'aic8800_fdrv', 'aic_zlp_quirk')}
    if set(modules) != wanted:
        raise ValueError('Missing production AIC modules')
    firmware = driver.get('firmware', [])
    if not firmware or any(not p.startswith('firmware/aic8800') for p in firmware):
        raise ValueError('Missing production AIC firmware')
    expected_paths = {'lib/' + p for p in modules + firmware}
    expected_paths.add('etc/udev/rules.d/90-carlink-aic8800.rules')
    if set(checks) != expected_paths:
        raise ValueError('Production AIC payload manifest differs')
    for relative, digest in checks.items():
        if not re.fullmatch('[0-9a-f]{64}', digest) or hashlib.sha256(payload(root, relative).read_bytes()).hexdigest() != digest:
            raise ValueError('Production AIC checksum differs: ' + relative)
    for relative in modules:
        data = payload(root, 'lib/' + relative).read_bytes()
        if len(data)<52 or data[:6]!=b'\x7fELF\x01\x01' or struct.unpack_from('<H', data,18)[0]!=40 or b'vermagic='+kernel.encode()+b' ' not in data:
            raise ValueError('Production AIC module ABI/kernel differs')
    payload(root, 'boot/config-' + kernel)
    payload(root, 'usr/sbin/usb_modeswitch')
    config = payload(root, 'etc/modprobe.d/carlink-aic.conf').read_text(encoding='utf-8')
    if 'options aic8800_fdrv country_code=CN custregd=0' not in config.splitlines():
        raise ValueError('Production AIC region config differs')
    print('Production manager, native HiCar and matched AIC payloads checked')


if __name__ == '__main__':
    if len(sys.argv) != 2:raise SystemExit('Usage: verify-production-runtime.py MOUNTED_ROOT')
    verify(sys.argv[1])
