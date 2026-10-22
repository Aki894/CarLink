#!/usr/bin/env bash
# Build only on Linux CI. Runtime identities are supplied separately on device.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
work="$root/.build/carplay"
out="$root/dist/carplay"
mkdir -p "$work"
mapfile -t lock < <(python3 - "$root/sources.lock.json" <<'PY'
import json, sys
x=json.load(open(sys.argv[1]))['carplay']
for key in ['rust','target']: print(x[key])
for key in ['relay','stream']:
    print(x[key]['repository']); print(x[key]['commit'])
PY
)
version=${lock[0]}; target=${lock[1]}
[[ "$target" == armv7-unknown-linux-gnueabihf ]]
checkout() {
    local destination=$1 url=$2 commit=$3
    [[ "$commit" =~ ^[0-9a-f]{40}$ ]]
    # A fresh worktree keeps patch application deterministic on runner retries.
    [[ ! -e "$destination" ]] || { echo "Build directory already exists: $destination" >&2; exit 1; }
    git init -q "$destination"
    git -C "$destination" remote add origin "$url"
    git -C "$destination" fetch --quiet --depth=1 origin "$commit"
    git -C "$destination" checkout --quiet --detach FETCH_HEAD
    [[ $(git -C "$destination" rev-parse HEAD) == "$commit" ]]
}
relay="$work/OpenXPlay-Relay"; stream="$work/OpenXPlay-Stream"
checkout "$relay" "${lock[2]}" "${lock[3]}"
checkout "$stream" "${lock[4]}" "${lock[5]}"
[[ $(tr -d '[:space:]' < "$relay/stream.lock") == "${lock[5]}" ]]
[[ ! -e "$out" ]] || { echo 'CarPlay output directory already exists' >&2; exit 1; }
mkdir -p "$out/bin" "$out/share/licenses/relay" "$out/share/licenses/stream" "$out/share/source/patches"
shopt -s nullglob
patches=("$root"/receiver/patches/*.patch)
for patch in "${patches[@]}"; do
    git -C "$relay" apply --check "$patch"
    git -C "$relay" apply "$patch"
    cp "$patch" "$out/share/source/patches/"
done

export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc
export CC_armv7_unknown_linux_gnueabihf=arm-linux-gnueabihf-gcc
export AR_armv7_unknown_linux_gnueabihf=arm-linux-gnueabihf-ar
export CARGO_TARGET_DIR="$work/target"
rustup toolchain install "$version" --profile minimal --target "$target"
# ring requires the cross C compiler; all other target-specific dependencies
# are resolved by the upstream Cargo.lock files rather than edited locally.
cargo +"$version" test --locked --manifest-path "$relay/rust/Cargo.toml" -p carplay-iap2 -p carplay-airplay
cargo +"$version" test --locked --manifest-path "$relay/rust/Cargo.toml" -p openxplay-relay carlink_capture_detects_delayed_sender_frames
cargo +"$version" build --locked --release --manifest-path "$relay/rust/Cargo.toml" --target "$target" -p openxplay-relay --bin openxplay-relay
cargo +"$version" build --locked --release --manifest-path "$stream/rust/Cargo.toml" --target "$target" -p oxp-bin --bin oxp-relay
for name in openxplay-relay oxp-relay; do
    install -m 0755 "$CARGO_TARGET_DIR/$target/release/$name" "$out/bin/"
    arm-linux-gnueabihf-readelf -h -A "$out/bin/$name" > "$out/$name.elf.txt"
    grep -q 'Class:.*ELF32' "$out/$name.elf.txt"
    grep -q 'Machine:.*ARM' "$out/$name.elf.txt"
    grep -q 'Tag_ABI_VFP_args:.*VFP registers' "$out/$name.elf.txt"
    qemu-arm -L /usr/arm-linux-gnueabihf "$out/bin/$name" --help > "$out/$name.help.txt"
done
# The upstream receiver launches oxp-relay and serves both web frontends.
# Keep them available even when CarLink's direct media bridge is selected.
(cd "$relay/admin-web" && npm ci && npm run build)
(cd "$stream/rust/web-receiver" && npm ci && npm run build)
cp -a "$relay/admin-web/dist" "$out/share/admin"
cp -a "$stream/rust/web-receiver/dist" "$out/share/web"
test -s "$out/share/web/receiver/index.html"
test -s "$out/share/admin/index.html"
mkdir -p "$out/share/plugins"
install -m 0755 "$root/scripts/run-carplay.sh" "$out/run-carplay.sh"
install -m 0755 "$root/receiver/probe.py" "$out/probe.py"
install -m 0755 "$root/receiver/forward.py" "$root/receiver/supervisor.py" "$out/"
install -m 0755 "$root/scripts/run-carlink.sh" "$out/run-carlink.sh"
bash "$root/scripts/build-audio.sh"
cp -a "$root/dist/audio/bin/." "$out/bin/"
cp -a "$root/dist/audio/share/." "$out/share/"
cp "$root/dist/audio/SOURCE.json" "$out/share/source/audio/SOURCE.json"
cp "$root/dist/audio/audio-validation.txt" "$root/dist/audio/audio.version.txt" "$root/dist/audio/audio.elf.txt" "$out/share/source/audio/"
for name in LICENSE LICENSE-GPL-3.0 NOTICE; do cp "$relay/$name" "$out/share/licenses/relay/"; done
cp -a "$relay/LICENSES" "$out/share/licenses/relay/"
for name in LICENSE NOTICE THIRD-PARTY-LICENSES.md; do cp "$stream/$name" "$out/share/licenses/stream/"; done
# Base source archives plus the ordered patches constitute the exact project
# sources. Cargo/npm lockfiles inside identify downloadable dependency sources.
git -C "$relay" archive --format=tar.gz HEAD > "$out/share/source/relay.tar.gz"
git -C "$stream" archive --format=tar.gz HEAD > "$out/share/source/stream.tar.gz"
cp "$root/scripts/build-carplay.sh" "$root/sources.lock.json" "$out/share/source/"
python3 - "$root" "$out" <<'PY'
import hashlib,json,pathlib,subprocess,sys
root,out=map(pathlib.Path,sys.argv[1:]); lock=json.loads((root/'sources.lock.json').read_text())['carplay']
manifest=dict(lock, project_commit=subprocess.check_output(['git','-C',str(root),'rev-parse','HEAD'],text=True).strip(),
    binaries=['bin/openxplay-relay','bin/oxp-relay'],
    patches=[{'file':'share/source/patches/'+p.name,'sha256':hashlib.sha256(p.read_bytes()).hexdigest()} for p in sorted((out/'share/source/patches').glob('*.patch'))],
    source_instructions='Extract relay.tar.gz and stream.tar.gz as sibling OpenXPlay-Relay and OpenXPlay-Stream directories. Apply listed patches in order to Relay. Run build-carplay.sh in the CarLink revision listed here; Cargo/npm locked dependencies are fetched from their recorded sources.',
    runtime_identity='Supplied locally; excluded from build and firmware.',
    license_notice='Powered by OpenXPlay. https://github.com/OpenXPlay/OpenXPlay-Relay')
(out/'SOURCE.json').write_text(json.dumps(manifest,indent=2)+'\n')
with (out/'SHA256SUMS').open('w') as sums:
    for p in sorted(out.rglob('*')):
        if p.is_file() and p.name!='SHA256SUMS': sums.write(hashlib.sha256(p.read_bytes()).hexdigest()+'  '+p.relative_to(out).as_posix()+'\n')
PY
(cd "$out" && sha256sum -c SHA256SUMS)
