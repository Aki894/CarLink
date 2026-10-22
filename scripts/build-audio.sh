#!/usr/bin/env bash
# Run on the Linux CI runner; never compile on the development PC or board.
# Audio-only FFmpeg, built from unmodified LGPL source with no external codecs.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
work="$root/.build/audio"
out="$root/dist/audio"
mapfile -t lock < <(python3 - "$root/sources.lock.json" <<'PY'
import json, sys
x = json.load(open(sys.argv[1]))['audio_transcoder']
for k in ['repository', 'commit', 'version', 'target', 'cpu']:
    print(x[k])
PY
)
url=${lock[0]}; commit=${lock[1]}; version=${lock[2]}
target=${lock[3]}; cpu=${lock[4]}
[[ "$commit" =~ ^[0-9a-f]{40}$ ]]
[[ "$target" == arm-linux-gnueabihf && "$cpu" == cortex-a7 ]]
[[ ! -e "$work" && ! -e "$out" ]] || { echo 'Audio build/output directory already exists' >&2; exit 1; }
for tool in git make python3 "$target-gcc" "$target-strip" "$target-readelf" qemu-arm; do
    command -v "$tool" >/dev/null
done
mkdir -p "$work" "$out/bin" "$out/share/licenses/audio" "$out/share/source/audio"
source_dir="$work/FFmpeg"
git init -q "$source_dir"
git -C "$source_dir" remote add origin "$url"
git -C "$source_dir" fetch --quiet --depth=1 origin "$commit"
git -C "$source_dir" checkout --quiet --detach FETCH_HEAD
[[ $(git -C "$source_dir" rev-parse HEAD) == "$commit" ]]

# Keep this self-contained configure command in the source bundle as well.
# --disable-everything removes every codec, format, filter and protocol before
# the narrow audio allowlist below. Libraries link into one small executable.
cat > "$work/configure-audio.sh" <<'CONFIGURE'
#!/usr/bin/env bash
set -euo pipefail
./configure \
    --enable-cross-compile --cross-prefix=arm-linux-gnueabihf- \
    --arch=arm --cpu=cortex-a7 --target-os=linux \
    --extra-cflags='-mfloat-abi=hard -mfpu=neon-vfpv4 -ffunction-sections -fdata-sections' \
    --extra-ldflags='-Wl,--gc-sections' --optflags=-Os \
    --disable-autodetect --disable-network --disable-everything \
    --disable-programs --enable-ffmpeg --disable-doc --disable-debug \
    --disable-shared --enable-static --enable-small \
    --disable-avdevice --disable-swscale \
    --disable-hwaccels --disable-gpl --disable-nonfree --disable-version3 \
    --enable-avcodec --enable-avformat --enable-avfilter --enable-swresample \
    --enable-decoder=aac,pcm_s16le,opus --enable-encoder=pcm_s16le \
    --enable-parser=aac,opus --enable-demuxer=aac,pcm_s16le,ogg \
    --enable-muxer=pcm_s16le --enable-protocol=pipe \
    --enable-filter=aresample,aformat,anull,abuffer,abuffersink
CONFIGURE
chmod +x "$work/configure-audio.sh"
(cd "$source_dir" && bash "$work/configure-audio.sh" && make -j"${CARLINK_AUDIO_JOBS:-$(nproc)}" ffmpeg)
install -m 0755 "$source_dir/ffmpeg" "$out/bin/carlink-audio"
"$target-strip" --strip-unneeded "$out/bin/carlink-audio"
"$target-readelf" -h -A "$out/bin/carlink-audio" > "$out/audio.elf.txt"
grep -q 'Class:.*ELF32' "$out/audio.elf.txt"
grep -q 'Machine:.*ARM' "$out/audio.elf.txt"
grep -q 'Tag_ABI_VFP_args:.*VFP registers' "$out/audio.elf.txt"
qemu-arm -L /usr/arm-linux-gnueabihf "$out/bin/carlink-audio" -version > "$out/audio.version.txt"
grep -q -- '--disable-gpl' "$out/audio.version.txt"
qemu-arm -L /usr/arm-linux-gnueabihf "$out/bin/carlink-audio" -L > "$out/share/licenses/audio/runtime-license.txt"
grep -q 'Lesser General Public License' "$out/share/licenses/audio/runtime-license.txt"

# Execute the actual ARM binary in CI: 100 ms 48 kHz stereo sine through pipe
# input/output, resampled to the 44.1 kHz PCM used by the CarLife media channel.
python3 - "$out" <<'PY'
import array, math, pathlib, subprocess, sys
out = pathlib.Path(sys.argv[1])
samples = array.array('h')
for i in range(4800):
    v = round(9000 * math.sin(2 * math.pi * 440 * i / 48000))
    samples.extend((v, v))
if sys.byteorder != 'little':
    samples.byteswap()
result = subprocess.run([
    'qemu-arm', '-L', '/usr/arm-linux-gnueabihf', str(out / 'bin/carlink-audio'),
    '-hide_banner', '-loglevel', 'error', '-nostdin',
    '-f', 's16le', '-ar', '48000', '-ac', '2', '-i', 'pipe:0',
    '-map', '0:a:0', '-c:a', 'pcm_s16le', '-ar', '44100', '-ac', '2',
    '-f', 's16le', 'pipe:1'], input=samples.tobytes(), capture_output=True,
    check=True, timeout=60)
pcm = array.array('h')
pcm.frombytes(result.stdout)
if sys.byteorder != 'little':
    pcm.byteswap()
assert len(result.stdout) == 4410 * 2 * 2, len(result.stdout)
assert max(map(abs, pcm)) > 8000
assert all(pcm[i] == pcm[i + 1] for i in range(0, len(pcm), 2))
(out / 'audio-validation.txt').write_text(
    'ARMv7 QEMU: pipe s16le 48000 Hz stereo -> 44100 Hz stereo passed\n'
    'Output: 4410 frames, 17640 bytes, non-silent equal stereo channels\n')
PY
for name in LICENSE.md COPYING.LGPLv2.1; do
    cp "$source_dir/$name" "$out/share/licenses/audio/"
done
git -C "$source_dir" archive --format=tar.gz --prefix="FFmpeg-$version/" HEAD > "$out/share/source/audio/ffmpeg.tar.gz"
cp "$work/configure-audio.sh" "$root/scripts/build-audio.sh" "$root/sources.lock.json" "$out/share/source/audio/"
cp "$source_dir/config.h" "$source_dir/config_components.h" "$out/share/source/audio/"
python3 - "$root" "$out" <<'PY'
import hashlib, json, pathlib, subprocess, sys
root, out = map(pathlib.Path, sys.argv[1:])
lock = json.loads((root / 'sources.lock.json').read_text())['audio_transcoder']
manifest = dict(lock,
    project_commit=subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip(),
    binary='bin/carlink-audio',
    components={'decoders': ['aac', 'pcm_s16le', 'opus'], 'encoders': ['pcm_s16le'],
                'demuxers': ['aac', 'pcm_s16le', 'ogg'], 'muxers': ['pcm_s16le'],
                'protocols': ['pipe']},
    source_instructions='Extract share/source/audio/ffmpeg.tar.gz. Run configure-audio.sh in its FFmpeg directory with the arm-linux-gnueabihf cross toolchain, then make ffmpeg. build-audio.sh reproduces and validates the full artifact from this pinned source.',
    attribution='FFmpeg https://ffmpeg.org/; LGPL-2.1-or-later. No GPL/nonfree/external codec components.')
(out / 'SOURCE.json').write_text(json.dumps(manifest, indent=2) + '\n')
with (out / 'SHA256SUMS').open('w') as sums:
    for p in sorted(out.rglob('*')):
        if p.is_file() and p.name != 'SHA256SUMS':
            sums.write(hashlib.sha256(p.read_bytes()).hexdigest() + '  ' + p.relative_to(out).as_posix() + '\n')
PY
(cd "$out" && sha256sum -c SHA256SUMS)
echo "Audio artifact ready: $out"
