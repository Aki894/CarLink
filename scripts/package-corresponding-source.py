"""Package only public, pinned build sources and expose notices beside the UI."""
import hashlib
import html
import io
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile


def package(root, output):
    root, output = Path(root).resolve(), Path(output).resolve()
    compliance = output / 'share/compliance'
    compliance.mkdir(parents=True, exist_ok=True)
    commit = subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip()
    # git archive excludes ignored reference repositories and device state.
    original = subprocess.check_output(['git', '-C', str(root), 'archive', '--format=tar', '--prefix=CarLink/', commit])
    archive = compliance / 'corresponding-source.tar.gz'
    with tarfile.open(archive, 'w:gz') as bundle:
        with tarfile.open(fileobj=io.BytesIO(original)) as tracked:
            for member in tracked:
                bundle.addfile(member, tracked.extractfile(member) if member.isfile() else None)
        for directory in ('source', 'licenses'):
            bundle.add(output / 'share' / directory, arcname='receiver-build/' + directory)
        bundle.add(output / 'SOURCE.json', arcname='receiver-build/SOURCE.json')
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    (compliance / 'SHA256SUMS').write_text(digest + '  corresponding-source.tar.gz\n', encoding='utf-8')
    lock = json.loads((root / 'sources.lock.json').read_text(encoding='utf-8'))['carplay']
    text = ['<!doctype html><html lang="en"><meta charset="utf-8"><title>CarLink open-source licenses</title>',
            '<h1>About / Open-source licenses · 关于 / 开源许可</h1>',
            '<p>Powered by OpenXPlay. <a href="https://openxplay.org/">OpenXPlay.org</a> · '
            '<a href="https://github.com/OpenXPlay/OpenXPlay-Relay">OpenXPlay-Relay</a> · '
            '<a href="https://github.com/OpenXPlay/OpenXPlay-Stream">OpenXPlay-Stream</a></p>',
            '<p>CarLink modifications / 修改版本: <code>' + commit + '</code></p>',
            '<p><a href="corresponding-source.tar.gz">Download exact Corresponding Source / 下载本版本对应源码</a></p>',
            '<p>SHA-256: <code>' + digest + '</code></p>',
            '<p>The archive includes pinned upstream source archives, ordered patches, build scripts, '
            'lockfiles, notices and the CarLink revision. Extract receiver-build/source/relay.tar.gz and '
            'stream.tar.gz into sibling OpenXPlay-Relay and OpenXPlay-Stream directories; apply patches '
            'in order (files ending -stream.patch apply to Stream). See CarLink/scripts/build-carplay.sh '
            'and receiver-build/SOURCE.json for the locked toolchain and build instructions. '
            'Cargo/npm fetch dependencies identified by the included lockfiles.</p>',
            '<p>Relay: ' + lock['relay']['commit'] + '<br>Stream: ' + lock['stream']['commit'] + '</p>']
    for path in [root / 'LICENSING.md', *sorted((root / 'LICENSES').glob('*'))]:
        if path.is_file():
            text.append('<h2>' + html.escape(path.name) + '</h2><pre>' + html.escape(path.read_text(encoding='utf-8')) + '</pre>')
    text.append('</html>')
    (compliance / 'index.html').write_text('\n'.join(text), encoding='utf-8')
    for destination in (output / 'share/admin/carlink-source', output / 'share/web/receiver/carlink-source'):
        shutil.copytree(compliance, destination, dirs_exist_ok=True)
    print(json.dumps({'commit': commit, 'source_sha256': digest, 'source_bytes': archive.stat().st_size}))


if __name__ == '__main__':
    package(*sys.argv[1:3])
