"""Verify the offered archive against the receiver artifact and pinned patches."""
import hashlib
import json
from pathlib import Path
import sys
import tarfile


def verify(output):
    output = Path(output)
    compliance = output / 'share/compliance'
    archive = compliance / 'corresponding-source.tar.gz'
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    assert (compliance / 'SHA256SUMS').read_text().split()[0] == digest
    manifest = json.loads((output / 'SOURCE.json').read_text())
    with tarfile.open(archive) as bundle:
        names = set(bundle.getnames())
        assert 'CarLink/scripts/build-carplay.sh' in names
        assert 'CarLink/LICENSES/AGPL-3.0-only.txt' in names
        assert 'CarLink/LICENSES/GPL-3.0-only.txt' in names
        for name in names:
            assert not name.startswith('/') and '..' not in Path(name).parts
            assert not name.startswith(('CarLink/research/', 'CarLink/.build/'))
        for relative in ('source/relay.tar.gz', 'source/stream.tar.gz', 'licenses/relay/NOTICE',
                         'licenses/stream/NOTICE', 'source/patches/MODIFICATIONS.md'):
            assert bundle.extractfile('receiver-build/' + relative).read() == (output / 'share' / relative).read_bytes()
        assert json.load(bundle.extractfile('receiver-build/SOURCE.json')) == manifest
        for patch in manifest['patches']:
            data = bundle.extractfile('receiver-build/' + patch['file'].removeprefix('share/')).read()
            assert hashlib.sha256(data).hexdigest() == patch['sha256']
    for page in ('share/compliance', 'share/admin/carlink-source', 'share/web/receiver/carlink-source'):
        directory = output / page
        assert hashlib.sha256((directory / archive.name).read_bytes()).hexdigest() == digest
        text = (directory / 'index.html').read_text(encoding='utf-8')
        for expected in ('Powered by OpenXPlay.', 'corresponding-source.tar.gz', manifest['project_commit'], digest):
            assert expected in text
    print('Corresponding source, notices, patches and all three download copies verified.')


if __name__ == '__main__':
    verify(sys.argv[1])
