#!/usr/bin/env bash
# Run only in Linux CI: pinned ARMv7 Opus microphone encoder and decoder check.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
work="$root/.build/opus"
out="$root/dist/opus"
mapfile -t lock < <(python3 - "$root/sources.lock.json" <<'PY'
import json, sys
x = json.load(open(sys.argv[1]))['opus_microphone']
for k in ('repository', 'commit', 'version', 'target', 'cpu'):
    print(x[k])
PY
)
url=${lock[0]}; commit=${lock[1]}; version=${lock[2]}; target=${lock[3]}; cpu=${lock[4]}
[[ "$commit" =~ ^[0-9a-f]{40}$ ]]
[[ "$target" == arm-linux-gnueabihf && "$cpu" == cortex-a7 ]]
[[ ! -e "$work" && ! -e "$out" ]] || { echo 'Opus build/output directory already exists' >&2; exit 1; }
for tool in git cmake python3 "$target-gcc" "$target-strip" "$target-readelf" qemu-arm; do
    command -v "$tool" >/dev/null
done
mkdir -p "$work" "$out/lib" "$out/share/licenses/opus" "$out/share/source/opus"
source_dir="$work/opus"
git init -q "$source_dir"
git -C "$source_dir" remote add origin "$url"
git -C "$source_dir" fetch --quiet --depth=1 origin "$commit"
git -C "$source_dir" checkout --quiet --detach FETCH_HEAD
[[ $(git -C "$source_dir" rev-parse HEAD) == "$commit" ]]
# Derive version metadata from the lock even though the checkout is depth-one.
git -C "$source_dir" tag "v$version" "$commit"

cat > "$work/configure-opus.sh" <<'CONFIGURE'
#!/usr/bin/env bash
set -euo pipefail
# Invoke in the extracted upstream source directory. No host libraries are used.
version=${1:?Usage: configure-opus.sh LOCKED_VERSION}
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
printf 'PACKAGE_VERSION="%s"\n' "$version" > package_version
cmake -S . -B build-armv7 \
    -DCMAKE_SYSTEM_NAME=Linux -DCMAKE_SYSTEM_PROCESSOR=arm \
    -DCMAKE_C_COMPILER=arm-linux-gnueabihf-gcc \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_C_FLAGS='-mcpu=cortex-a7 -mfloat-abi=hard -mfpu=neon-vfpv4 -Os' \
    -DOPUS_BUILD_SHARED_LIBRARY=ON -DOPUS_BUILD_PROGRAMS=OFF \
    -DOPUS_BUILD_TESTING=OFF -DOPUS_CUSTOM_MODES=OFF \
    -DOPUS_DRED=OFF -DOPUS_OSCE=OFF \
    -DOPUS_INSTALL_PKG_CONFIG_MODULE=OFF -DOPUS_INSTALL_CMAKE_CONFIG_MODULE=OFF
cmake --build build-armv7 --parallel "${CARLINK_OPUS_JOBS:-2}" --target opus
CONFIGURE
chmod +x "$work/configure-opus.sh"
(cd "$source_dir" && bash "$work/configure-opus.sh" "$version")
install -m 0644 "$(readlink -f "$source_dir/build-armv7/libopus.so")" "$out/lib/libcarlink_opus.so"
"$target-strip" --strip-unneeded "$out/lib/libcarlink_opus.so"
"$target-readelf" -h -A "$out/lib/libcarlink_opus.so" > "$out/share/source/opus/opus.elf.txt"
grep -q 'Class:.*ELF32' "$out/share/source/opus/opus.elf.txt"
grep -q 'Machine:.*ARM' "$out/share/source/opus/opus.elf.txt"
grep -q 'Tag_ABI_VFP_args:.*VFP registers' "$out/share/source/opus/opus.elf.txt"

# dlopen the exact renamed runtime payload rather than a host/system libopus.
cat > "$work/opus-smoke.c" <<'C'
#include <dlfcn.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include "opus.h"
#define LOAD(name) __typeof__(&name) p_##name = (__typeof__(&name))dlsym(lib, #name); if (!p_##name) return 2
int main(int argc, char **argv) {
    if (argc != 2) return 1;
    void *lib = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL);
    if (!lib) { fprintf(stderr, "%s\n", dlerror()); return 2; }
    LOAD(opus_encoder_create); LOAD(opus_encoder_ctl); LOAD(opus_encode);
    LOAD(opus_encoder_destroy); LOAD(opus_decoder_create); LOAD(opus_decode);
    LOAD(opus_decoder_destroy); LOAD(opus_packet_get_nb_samples); LOAD(opus_get_version_string);
    for (int channels = 1; channels <= 2; channels++) {
        int error = 0;
        OpusEncoder *encoder = p_opus_encoder_create(48000, channels, OPUS_APPLICATION_VOIP, &error);
        if (!encoder || error) return 3;
        OpusDecoder *decoder = p_opus_decoder_create(48000, channels, &error);
        if (!decoder || error) return 4;
        if (p_opus_encoder_ctl(encoder, OPUS_SET_BITRATE(32000 * channels)) ||
            p_opus_encoder_ctl(encoder, OPUS_SET_COMPLEXITY(3))) return 5;
        opus_int16 pcm[1920], decoded[1920]; unsigned char packet[1275];
        long long energy = 0;
        for (int frame = 0; frame < 25; frame++) {
            for (int i = 0; i < 960; i++) {
                opus_int16 value = (opus_int16)(9000 * sin(2 * 3.141592653589793 * 440 * (frame * 960 + i) / 48000));
                for (int c = 0; c < channels; c++) pcm[i * channels + c] = value;
            }
            int size = p_opus_encode(encoder, pcm, 960, packet, sizeof(packet));
            if (size <= 0 || p_opus_packet_get_nb_samples(packet, size, 48000) != 960) return 6;
            if (p_opus_decode(decoder, packet, size, decoded, 960, 0) != 960) return 7;
            for (int i = 0; i < 960 * channels; i++) energy += abs(decoded[i]);
        }
        if (energy < 1000000) return 8;
        p_opus_encoder_destroy(encoder); p_opus_decoder_destroy(decoder);
    }
    printf("ARMv7 QEMU: %s, 48000 Hz mono/stereo, 25 x 20 ms encode/decode passed\n", p_opus_get_version_string());
    dlclose(lib);
    return 0;
}
C
"$target-gcc" -mcpu=cortex-a7 -mfloat-abi=hard -mfpu=neon-vfpv4 -Os \
    -I "$source_dir/include" "$work/opus-smoke.c" -ldl -lm -o "$work/opus-smoke"
qemu-arm -L /usr/arm-linux-gnueabihf "$work/opus-smoke" "$out/lib/libcarlink_opus.so" \
    | tee "$out/share/source/opus/opus-validation.txt"
grep -q '20 ms encode/decode passed' "$out/share/source/opus/opus-validation.txt"
cp "$source_dir/COPYING" "$out/share/licenses/opus/COPYING"
git -C "$source_dir" archive --format=tar.gz --prefix="opus-$version/" HEAD > "$out/share/source/opus/opus.tar.gz"
cp "$work/configure-opus.sh" "$work/opus-smoke.c" "$root/scripts/build-opus.sh" "$out/share/source/opus/"
cp "$root/sources.lock.json" "$out/share/source/opus/"
python3 - "$root" "$out" <<'PY'
import hashlib, json, pathlib, subprocess, sys
root, out = map(pathlib.Path, sys.argv[1:])
source = dict(json.loads((root/'sources.lock.json').read_text())['opus_microphone'],
    project_commit=subprocess.check_output(['git','-C',str(root),'rev-parse','HEAD'],text=True).strip(),
    binary='lib/libcarlink_opus.so',
    validation='ARMv7 QEMU: real mono/stereo 48000 Hz 20 ms encode/decode',
    source_instructions='Extract opus.tar.gz; run configure-opus.sh with the locked version as its argument in the source directory. build-opus.sh reproduces the renamed ARM hard-float shared library and executes opus-smoke.c with QEMU.',
    attribution='Opus https://opus-codec.org/; Xiph.Org and contributors; BSD-3-Clause. See share/licenses/opus/COPYING.')
(out/'share/source/opus/SOURCE.json').write_text(json.dumps(source,indent=2)+'\n')
with (out/'SHA256SUMS').open('w') as sums:
    for path in sorted(out.rglob('*')):
        if path.is_file() and path.name != 'SHA256SUMS':
            sums.write(hashlib.sha256(path.read_bytes()).hexdigest()+'  '+path.relative_to(out).as_posix()+'\n')
PY
(cd "$out" && sha256sum -c SHA256SUMS)
echo "Opus microphone artifact ready: $out"
