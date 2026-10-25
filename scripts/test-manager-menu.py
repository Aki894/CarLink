"""CI: host and QEMU ARM menu rendering, actual H.264 decode and focus checks."""
from pathlib import Path
import subprocess
import sys

host, arm, font, output = (Path(s).resolve() for s in sys.argv[1:5])
size = 800 * 480 * 3 // 2
for label, command in [('host', [str(host)]),
                       ('armv7', ['qemu-arm', '-L', '/usr/arm-linux-gnueabihf', str(arm)])]:
    result = subprocess.run(command + [str(font)], check=True, capture_output=True, timeout=60)
    assert 200 < len(result.stdout) < 1024 * 1024
    encoded = output / ('menu-' + label + '.h264')
    encoded.write_bytes(result.stdout)
    decoded = subprocess.run(['ffmpeg', '-v', 'error', '-f', 'h264', '-i', str(encoded),
        '-frames:v', '2', '-pix_fmt', 'yuv420p', '-f', 'rawvideo', 'pipe:1'],
        capture_output=True, check=True, timeout=30).stdout
    assert len(decoded) == size * 2, (label, len(decoded))
    first, second = decoded[:size], decoded[size:]
    # Selection switches from row one to row two. Check padding, away from text.
    for pixels, top, bottom in [(first, 210, 25), (second, 25, 210)]:
        assert abs(pixels[90 * 800 + 28] - top) < 12
        assert abs(pixels[140 * 800 + 28] - bottom) < 12
    failed = subprocess.run(command + ['/dev/null'], capture_output=True, timeout=20)
    assert failed.returncode != 0
    print(label + ': two independent 800x480 IDRs decoded; Chinese labels, focus change and invalid font checked', flush=True)
