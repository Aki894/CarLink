"""Generate public synthetic 2-second CarLife assets; requires local ffmpeg.

Example: python scripts/generate-test-media.py .build/test-media --width 800 --height 480 --fps 30
No generated binary assets belong in Git. The application sends these only when
its caller explicitly enables test media and the head unit starts video.
"""
import argparse
import hashlib
import json
import pathlib
import shutil
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=pathlib.Path)
    parser.add_argument("--width", type=int, default=800)
    parser.add_argument("--height", type=int, default=480)
    parser.add_argument("--fps", type=int, default=30)
    parser.add_argument("--ffmpeg", default="ffmpeg")
    parser.add_argument("--overwrite", action="store_true", help="replace existing generated files")
    args = parser.parse_args()
    if not (16 <= args.width <= 1920 and 16 <= args.height <= 1080
            and args.width % 2 == args.height % 2 == 0 and 1 <= args.fps <= 60):
        parser.error("need even dimensions 16..1920 x 16..1080 and fps 1..60")
    executable = shutil.which(args.ffmpeg)
    if executable is None:
        parser.error("ffmpeg was not found")
    names = ["video.conf", "video.h264", "tone.pcm", "media-manifest.json", "SHA256SUMS"]
    if not args.overwrite and any((args.directory / name).exists() for name in names):
        parser.error("output already contains media assets; choose another directory or --overwrite")
    args.directory.mkdir(parents=True, exist_ok=True)
    video = args.directory / "video.h264"
    audio = args.directory / "tone.pcm"
    # x264 repeat-headers plus AUD allows exact access-unit splitting. The
    # Rust packet source strips AUD/SEI and repeat SPS/PPS after the first AU.
    video_cmd = [executable, "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi",
                 "-i", f"testsrc2=size={args.width}x{args.height}:rate={args.fps}:duration=2",
                 "-an", "-c:v", "libx264", "-profile:v", "baseline", "-pix_fmt", "yuv420p",
                 "-preset", "veryfast", "-tune", "zerolatency", "-b:v", "1000k",
                 "-bf", "0", "-g", str(args.fps), "-keyint_min", str(args.fps),
                 "-x264-params", "aud=1:repeat-headers=1:scenecut=0",
                 "-frames:v", str(args.fps * 2), "-f", "h264", str(video)]
    # Intermittent quiet tone: 100 ms in every 500 ms, with short edge fades.
    # aevalsrc expression uses seconds; stereo channels carry identical tone.
    wave = "0.025*sin(2*PI*440*t)*lt(mod(t,0.5),0.1)*min(1,mod(t,0.5)/0.005)*min(1,(0.1-mod(t,0.5))/0.005)"
    audio_cmd = [executable, "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi",
                 "-i", f"aevalsrc=exprs='{wave}|{wave}':s=44100:d=2",
                 "-ar", "44100", "-ac", "2", "-c:a", "pcm_s16le", "-f", "s16le", str(audio)]
    for command in (video_cmd, audio_cmd):
        subprocess.run(command, check=True)
    if not (0 < video.stat().st_size <= 16 * 1024 * 1024):
        raise RuntimeError("video output outside 16 MiB bound")
    if audio.stat().st_size != 44100 * 4 * 2:
        raise RuntimeError("PCM output must contain exactly 88200 stereo sample frames")
    (args.directory / "video.conf").write_text(f"{args.width} {args.height} {args.fps}\n", encoding="utf-8")
    version = subprocess.run([executable, "-version"], check=True, capture_output=True, text=True).stdout.splitlines()[0]
    manifest = {"format": 1, "duration_seconds": 2,
                "video": {"width": args.width, "height": args.height, "fps": args.fps,
                          "codec": "H264 Annex-B baseline", "b_frames": 0, "aud": True,
                          "repeat_headers": True, "gop_frames": args.fps},
                "audio": {"sample_rate": 44100, "channels": 2, "format": "PCM16LE",
                          "tone_hz": 440, "peak_amplitude": 0.025},
                "ffmpeg": version, "commands": [video_cmd, audio_cmd]}
    (args.directory / "media-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    names = ["video.conf", "video.h264", "tone.pcm", "media-manifest.json"]
    sums = [f"{hashlib.sha256((args.directory / name).read_bytes()).hexdigest()}  {name}\n" for name in names]
    (args.directory / "SHA256SUMS").write_text("".join(sums), encoding="ascii")
    print(json.dumps({"directory": str(args.directory.resolve()), "files": names + ["SHA256SUMS"]}))


if __name__ == "__main__":
    try:
        main()
    except (OSError, subprocess.CalledProcessError, RuntimeError) as error:
        print(f"media generation failed: {error}", file=sys.stderr)
        sys.exit(1)
