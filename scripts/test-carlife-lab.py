"""Independent stdlib-only HU fixture for the CarLife MD lab transport.

Fixtures use the public CarLife proto2 field numbers and USB framing. This is
not a wireless CarLife HU. --address also runs against a board via SSH forwarding.
"""
import argparse
import json
import queue
import socket
import struct
import subprocess
import threading
import time


def command(kind, payload=b"", channel=1):
    inner = struct.pack(">HHI", len(payload), 0, kind) + payload
    return struct.pack(">II", channel, len(inner)) + inner


def exact(stream, count):
    chunks = bytearray()
    while len(chunks) < count:
        part = stream.recv(count - len(chunks))
        if not part:
            raise AssertionError("unexpected EOF from MD")
        chunks.extend(part)
    return bytes(chunks)


def receive(stream):
    channel, length = struct.unpack(">II", exact(stream, 8))
    assert channel == 1 and 8 <= length <= 65543, (channel, length)
    data = exact(stream, length)
    size, reserved, kind = struct.unpack(">HHI", data[:8])
    assert reserved == 0 and size == len(data) - 8
    return kind, data[8:]


def no_response(stream):
    stream.settimeout(0.2)
    try:
        data = stream.recv(1)
    except socket.timeout:
        return
    raise AssertionError("unexpected response or EOF: " + repr(data))


def handshake(stream):
    # Reproduce a HU splitting the inner header and protobuf body into separate
    # outer frames, while TCP itself also fragments the 8-byte outer header.
    first = command(0x18001, b"\x08\x02\x10\x00")
    split = struct.pack(">II", 1, 8) + first[8:16]
    split += struct.pack(">II", 1, 4) + first[16:]
    for cut in (split[:1], split[1:5], split[5:11], split[11:]):
        stream.sendall(cut)
    stream.settimeout(3)
    responses = [receive(stream) for _ in range(4)]
    assert [kind for kind, _ in responses] == [0x10002, 0x1001B, 0x10018, 0x10004], responses
    assert responses[0][1] == b"\x08\x01", "match-status must be proto2 int field 1 = 1"
    assert responses[3][1], "device info missing"
    stream.sendall(first)
    no_response(stream)  # duplicate VERSION must not reset an active logical session
    stream.settimeout(3)
    # 1280x720 @30, independently encoded protobuf varints.
    config = b"\x08\x80\x0a\x10\xd0\x05\x18\x1e"
    stream.sendall(command(0x18007, config))
    kind, reply = receive(stream)
    assert kind == 0x10008 and reply == config, (kind, reply.hex())
    # PAUSE/RESET are deliberately coalesced with START into one stream write.
    stream.sendall(command(0x18009) + command(0x1800A) + command(0x1800B))
    no_response(stream)


def malformed(stream):
    stream.sendall(struct.pack(">II", 7, 8) + b"\0" * 8)


def truncated(stream):
    stream.sendall(command(0x18001, b"\x08\x02\x10\x00")[:-1])


def unsupported_auth(stream):
    # A challenge must produce a diagnostic error, never a fabricated success.
    stream.sendall(command(0x18001, b"\x08\x02\x10\x00"))
    for _ in range(4):
        receive(stream)
    stream.sendall(command(0x18048, b"\x0a\x04test"))


CASES = {"handshake": (handshake, 0), "malformed": (malformed, 1),
         "truncated": (truncated, 1), "unsupported-auth": (unsupported_auth, 1)}


def run_binary(binary, name):
    process = subprocess.Popen([binary, "carlife", "tcp", "--listen", "127.0.0.1:0", "--duration", "12"],
                               stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8")
    lines, events = [], queue.Queue()
    def collect():
        for line in process.stdout:
            lines.append(line.rstrip())
            if len(lines) > 1000:
                process.kill()
                return
            try:
                events.put(json.loads(line))
            except ValueError:
                pass
    reader = threading.Thread(target=collect, daemon=True)
    reader.start()
    try:
        limit = time.monotonic() + 8
        while True:
            event = events.get(timeout=max(0.01, limit - time.monotonic()))
            if event.get("event") == "tcp_listening":
                host, port = event["detail"].rsplit(":", 1)
                break
        with socket.create_connection((host, int(port)), timeout=3) as stream:
            CASES[name][0](stream)
            stream.shutdown(socket.SHUT_WR)
            # Drain to EOF to let the MD validate its final partial frame.
            stream.settimeout(3)
            try:
                while stream.recv(4096):
                    pass
            except ConnectionResetError:
                pass
        code = process.wait(timeout=5)
        reader.join(timeout=1)
        assert code == CASES[name][1], (name, code, lines)
        if name == "handshake":
            assert any("AwaitingVideoSource" in line for line in lines), lines
        if name == "unsupported-auth":
            assert not any("id=0x00010049" in line for line in lines), lines
        print(json.dumps({"case": name, "status": "passed", "exit_code": code}))
    except BaseException:
        print("\n".join(lines))
        raise
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=3)
        reader.join(timeout=1)
        process.stdout.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--binary")
    source.add_argument("--address", help="HOST:PORT; handshake only, running MD required")
    parser.add_argument("--case", choices=list(CASES) + ["all"], default="all")
    args = parser.parse_args()
    if args.binary:
        for name in CASES if args.case == "all" else [args.case]:
            run_binary(args.binary, name)
    else:
        host, port = args.address.rsplit(":", 1)
        with socket.create_connection((host, int(port)), timeout=3) as stream:
            handshake(stream)
            stream.shutdown(socket.SHUT_WR)
            stream.settimeout(3)
            assert stream.recv(1) == b"", "expected clean MD shutdown"
        print(json.dumps({"case": "board-handshake", "status": "passed"}))
