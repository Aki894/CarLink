"""Exercise the production identity CLI on host and ARM CI without radio access."""
import concurrent.futures
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile

command = json.loads(os.environ.get('CARLINK_RUST_RUNNER', '[]')) + [str(Path(sys.argv[1]).resolve()), 'hicar', 'identity-init']
with tempfile.TemporaryDirectory(prefix='carlink-identity-ci-') as folder:
    root = Path(folder)
    target = root / 'production/identity.json'

    def initialize(*args):
        result = subprocess.run(command + ['--identity', str(target), *args], capture_output=True, text=True, timeout=15)
        assert result.returncode == 0, 'production identity initialization failed'
        return json.loads(result.stdout), result.stdout + result.stderr

    with concurrent.futures.ThreadPoolExecutor(max_workers=6) as pool:
        replies = list(pool.map(lambda _: initialize(), range(6)))
    assert sum(reply['created'] for reply, _ in replies) == 1
    original = target.read_bytes()
    identity = json.loads(original)
    assert identity['schema'] == 1 and identity['name'] == 'CarLink'
    assert len(identity['device_id']) == 64 and all(c in '0123456789abcdef' for c in identity['device_id'])
    assert len(identity['pin']) == 6 and identity['pin'].isascii() and identity['pin'].isdigit()
    assert all(identity['device_id'] not in output and identity['pin'] not in output for _, output in replies)
    assert stat.S_IMODE(target.stat().st_mode) == 0o600
    assert stat.S_IMODE(target.parent.stat().st_mode) == 0o700
    keys = target.parent / 'keys'
    keys.mkdir()
    (keys / 'retained').write_bytes(b'existing native keys')
    assert initialize('--name', 'New name')[0]['created'] is False
    assert target.read_bytes() == original and (keys / 'retained').read_bytes() == b'existing native keys'

    invalid = root / 'invalid.json'
    invalid.write_bytes(b'invalid private data')
    invalid.chmod(0o600)
    for path in (invalid, root / 'linked.json'):
        if path != invalid:
            path.symlink_to(target)
        result = subprocess.run(command + ['--identity', str(path)], capture_output=True, timeout=15)
        assert result.returncode != 0
    assert invalid.read_bytes() == b'invalid private data' and target.read_bytes() == original
    assert not list(target.parent.glob('.identity-*'))
print(json.dumps({'production_identity_init': 'passed', 'concurrent_creators': 6, 'existing_identity_and_keys_preserved': True, 'private_data_logged': False}))
