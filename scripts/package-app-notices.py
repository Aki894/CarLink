"""Package locked Rust source archives and license notices with the application."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tomllib

project=Path(__file__).resolve().parents[1]
destination=Path(sys.argv[1]).resolve()
version=os.environ['RUST_VERSION']
metadata=json.loads(subprocess.check_output(['cargo','+'+version,'metadata','--locked','--format-version','1',
                    '--manifest-path',str(project/'app/Cargo.toml')],text=True))
locked={(p['name'],p['version']):p for p in tomllib.loads((project/'app/Cargo.lock').read_text())['package']}
notices=destination/'share/licenses/rust'
source=destination/'share/source/rust'
(source/'crates').mkdir(parents=True,exist_ok=True)
records=[]
for package in metadata['packages']:
    if not package.get('source'):
        continue
    if not package['source'].startswith('registry+'):
        raise ValueError('Rust dependency must use the locked registry')
    manifest=Path(package['manifest_path'])
    name=package['name'];release=package['version'];identifier=name+'-'+release
    archive=manifest.parents[3]/'cache'/manifest.parents[1].name/(identifier+'.crate')
    expected=locked[(name,release)]['checksum']
    if hashlib.sha256(archive.read_bytes()).hexdigest()!=expected:
        raise ValueError('Rust source checksum differs from registry lock: '+name)
    shutil.copyfile(archive,source/'crates'/archive.name)
    license_files=[]
    for path in sorted(manifest.parent.iterdir()):
        if path.is_file() and path.name.upper().startswith(('LICENSE','LICENCE','COPYING','COPYRIGHT','NOTICE')):
            target=notices/identifier/path.name
            target.parent.mkdir(parents=True,exist_ok=True)
            shutil.copyfile(path,target)
            license_files.append(path.name)
    if not license_files:
        raise ValueError('Dependency has no packaged license: '+name)
    records.append(dict(name=name,version=release,license=package['license'],sha256=expected,
                        source='https://crates.io/api/v1/crates/'+name+'/'+release+'/download',
                        license_files=license_files))
shutil.copyfile(project/'app/Cargo.lock',source/'Cargo.lock')
shutil.copyfile(project/'app/Cargo.toml',source/'Cargo.toml')
shutil.copytree(project/'app/src',source/'src',dirs_exist_ok=True)
notices.mkdir(parents=True,exist_ok=True)
(notices/'SOURCE.json').write_text(json.dumps(dict(toolchain=version,
    project_commit=os.environ['GITHUB_SHA'],dependencies=records),indent=2)+'\n')
lines=[]
for path in sorted(destination.rglob('*')):
    if path.is_file() and path.name!='SHA256SUMS':
        lines.append(hashlib.sha256(path.read_bytes()).hexdigest()+'  '+path.relative_to(destination).as_posix())
(destination/'SHA256SUMS').write_text('\n'.join(lines)+'\n')
print('Packaged locked Rust source and license notices:',len(records),'dependencies')
