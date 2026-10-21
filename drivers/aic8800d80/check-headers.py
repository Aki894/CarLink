"""Verify the actual headers package, including generated build ABI inputs."""
import hashlib
import io
import json
import pathlib
import sys
import tarfile
import zipfile

lock = json.loads((pathlib.Path(__file__).parent / 'sources.json').read_text())
location = pathlib.Path(sys.argv[1])
if location.is_file() and location.suffix == '.zip':
    with zipfile.ZipFile(location) as archive:
        names = [n for n in archive.namelist() if pathlib.PurePosixPath(n).name == lock['headers_package']]
        if len(names) != 1:
            raise SystemExit('Exact headers package missing or duplicated')
        package = archive.read(names[0])
else:
    names = list(location.rglob(lock['headers_package']))
    if len(names) != 1:
        raise SystemExit('Exact headers package missing or duplicated')
    package = names[0].read_bytes()
assert hashlib.sha256(package).hexdigest() == lock['headers_sha256'], 'Headers checksum mismatch'
assert package[:8] == b'!<arch>\n', 'Invalid Debian package'
offset = 8
payload = None
while offset < len(package):
    header = package[offset:offset + 60]
    size = int(header[48:58])
    name = header[:16].decode('ascii').strip().rstrip('/')
    contents = package[offset + 60:offset + 60 + size]
    offset += 60 + size + size % 2
    if name.startswith('data.tar'):
        payload = contents
assert payload is not None, 'No Debian data archive'
with tarfile.open(fileobj=io.BytesIO(payload), mode='r:*') as archive:
    prefix = './usr/src/linux-headers-' + lock['kernel_release'] + '/'
    for name, expected in [('Module.symvers', lock['symvers_sha256']),
                           ('.config', lock['kernel_config_sha256'])]:
        contents = archive.extractfile(prefix + name).read()
        assert hashlib.sha256(contents).hexdigest() == expected, name + ' mismatch'
    release = archive.extractfile(prefix + 'include/generated/utsrelease.h').read().decode()
    assert release.strip() == '#define UTS_RELEASE "' + lock['kernel_release'] + '"'
    for name in ['Makefile', 'include/config/auto.conf', 'scripts/basic/fixdep.c',
                 'scripts/mod/modpost.c', 'scripts/module.lds']:
        assert archive.getmember(prefix + name).isfile(), 'Missing header input: ' + name
print(json.dumps({'kernel_release': lock['kernel_release'], 'headers_sha256': lock['headers_sha256'],
                  'symvers_sha256': lock['symvers_sha256'], 'status': 'verified'}))
