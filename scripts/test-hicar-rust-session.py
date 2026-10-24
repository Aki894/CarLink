"""R2 state transcripts and private-IPC native authentication; synthetic identities only."""
import dataclasses
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import random
import socket
import struct
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'experiments/hicar'))
from nearby_wire import Frame, pack_tlv, pack_uint
from control import Negotiator
from network import Offer
from network_wire import Resource

spec = importlib.util.spec_from_file_location('hicar_codec', ROOT / 'scripts/test-hicar-rust.py')
codec = importlib.util.module_from_spec(spec)
spec.loader.exec_module(codec)


def transcript(binary):
    process = codec.Process([*json.loads(os.environ.get('CARLINK_RUST_RUNNER', '[]')),
                             binary, 'hicar', 'codec'])
    offer = Offer('CarLink', '02:00:00:00:00:01', 'synthetic-password')
    summary = b'A' * 64
    prefix = [Frame(0, 0, b'', pack_tlv(1, b'\x02')),
              Frame(0, 0, b'', pack_tlv(3, b'peer-summary')),
              Frame(0, 0, b'', pack_tlv(5, b'\x00'))]
    def capability(caps):
        body = b''.join(pack_uint(k) + pack_uint(v) for k, v in caps.items())
        return Frame(4, 0, b'', pack_tlv(1, pack_uint(3) + pack_tlv(3, body)))
    supported = capability({2: 1, 4: 1, 5: 511})
    selected = Frame(4, 0, b'', pack_tlv(7, pack_uint(3) + pack_uint(1) +
                                                       pack_tlv(3, b'\x02' + pack_uint(5745))))
    cases = []
    for authenticated in (False, True):
        cases.extend((authenticated, frames) for frames in [prefix,
            prefix + [supported, selected, Frame(33, 0, b'', pack_tlv(2, b'02:00:00:00:00:02'))],
            prefix + [Frame(2, 0, b'', pack_tlv(3, pack_uint(2) + pack_uint(19) + pack_uint(0) + pack_uint(0)))],
            prefix + [supported, Frame(4, 0, b'', pack_tlv(3, pack_uint(3) + pack_uint(20) + pack_uint(0) + pack_uint(1)))],
            prefix + [Frame(33, 0, b'', pack_tlv(2, b'02:00:00:00:00:02'))],
            prefix + [supported, Frame(33, 1, b'', pack_tlv(2, b'02:00:00:00:00:02'))],
            prefix + [supported, Frame(33, 0, b'', pack_tlv(2, b'not-a-MAC'))],
            prefix + [supported, Frame(33, 0, b'', pack_tlv(1, b''))],
            prefix + [supported, Frame(33, 0, b'', pack_tlv(5, b''))]])
    rng = random.Random(20261025)
    for _ in range(160):
        caps = {2: rng.randrange(3), 4: rng.randrange(3), 5: rng.randrange(512)}
        cases.append((True, prefix + [capability(caps)]))
    for payload in [b'', pack_tlv(1, b''), pack_tlv(1, b'\x01'),
                    pack_tlv(5, b'\x03'), pack_tlv(3, b''), b'\x81']:
        cases.append((True, [Frame(0, 0, b'', payload)]))
    try:
        for index, (authenticated, frames) in enumerate(cases):
            resource = Resource(offer)
            resource.authenticated = authenticated
            resource.data_port = 4006
            n = Negotiator(summary, 24, resource)
            expected = []
            try:
                for frame in frames:
                    reply = n.respond(frame)
                    expected.append(dict(reply=reply.hex() if reply is not None else None,
                        complete=n.complete, pending=[[k, p.hex()] for k, p in resource.take_pending()],
                        credentials_sent=resource.credentials_sent))
                success = True
            except (ValueError, UnicodeError):
                success = False
            result = process.call(dict(op='nearby-session', offer=dataclasses.asdict(offer),
                summary=summary.hex(), business=24, authenticated=authenticated,
                frames=[dict(kind=f.kind, flags=f.flags, payload=f.payload.hex()) for f in frames]))
            assert result.get('ok') is success, f'connection acceptance differs in case {index}'
            if success:
                assert result['result'] == dict(steps=expected), f'connection reply differs in case {index}'
    finally:
        process.close()
    print(json.dumps(dict(rust_connection_transcripts='passed', cases=len(cases))))


def authentication(binary, library, rust_library):
    from rust_session import Worker, AuthSession as RustAuth, Negotiator as RustNegotiator
    from identity import Identity
    from auth_wire import JsonStream, AuthError
    offer = Offer('CarLink', '02:00:00:00:00:01', 'synthetic-password')
    model, address = '00040A00', '02:11:22:33:44:55'
    stored = Identity('AB' * 32, '012345')
    own = stored.for_adapter(model, address).device_id
    client_id = 'CD' * 32
    runner = json.loads(os.environ.get('CARLINK_RUST_RUNNER', '[]'))
    results = []
    with tempfile.TemporaryDirectory(prefix='carlink-r2-') as directory:
        root = Path(directory)
        identity = root / 'identity.json'
        identity.write_bytes(stored.encoded())
        identity.chmod(0o600)
        original = identity.read_bytes()
        for phase in ('bind', 'auth'):
            w = p = auth = None
            try:
                w = Worker(binary, identity, model, address, rust_library, root / 'server',
                           offer, own.encode(), 24, runner=runner)
                resource = Resource(offer)
                # Prior to successful auth, even a negotiated data channel is gated.
                n = RustNegotiator(w, resource, True)
                for f in [Frame(0, 0, b'', pack_tlv(1, b'\x02') + pack_tlv(3, b'peer') + pack_tlv(5, b'\x00'))]:
                    n.respond(f)
                capability = b''.join(pack_uint(k) + pack_uint(v) for k, v in {2: 1, 4: 1, 5: 511}.items())
                network = Frame(4, 0, b'', pack_tlv(1, pack_uint(3) + pack_tlv(3, capability)))
                client_mac = Frame(33, 0, b'', pack_tlv(2, b'02:00:00:00:00:02'))
                assert n.respond(network) is None and n.respond(client_mac) is None
                auth = RustAuth(w)
                envelope = lambda op, data: (dict(version='14.2.0.400', oper_type=op, synctime=0,
                    trace_id='synthetic', data=json.dumps(data)) if phase == 'auth' else
                    dict(version='A_0', discover_type=1, oper_type=op, usb_connection=False, data=json.dumps(data)))
                reply, metadata = auth.receive(envelope(1, dict(deviceId=client_id, usb_connection=False)))
                assert metadata['operation'] == 1 and json.loads(json.loads(reply)['data'])['deviceId'] == own
                p = codec.Process([sys.executable, str(ROOT / 'scripts/test-hicar-rust-auth.py'), '--python-worker'])
                assert p.call(dict(library=library, store=str(root / 'client'), hardwareSeed=client_id,
                    own=client_id, peer=own, pin=stored.pin, role=1))['ready']
                state = p.call(dict(op='begin', phase=phase))
                pending = [(True, packet) for packet in state['packets']]
                for _ in range(32):
                    if not pending:
                        break
                    to_rust, packet = pending.pop(0)
                    if to_rust:
                        reply, metadata = auth.receive(envelope(3, json.loads(packet)))
                        if reply:
                            stream = JsonStream()
                            pending.extend((False, item['data']) for item in stream.feed(reply))
                            stream.finish()
                    else:
                        state = p.call(dict(op='receive', data=packet))
                        pending.extend((True, packet) for packet in state['packets'])
                assert not pending and 0 in state['results'] and metadata.get('key_ready')
                assert hashlib.sha256(auth.session_key).hexdigest() == state['key_hash']
                assert len(auth.session_key) == (32 if phase == 'bind' else 16)
                assert n.respond(network) is not None
                assert offer.credentials() in n.respond(client_mac)
                # A stale command must not revoke a newer active authentication.
                try:
                    w.call(dict(op='auth', generation=auth.generation + 1, envelope='{}'))
                except ValueError:
                    pass
                else:
                    raise AssertionError('stale authentication generation accepted')
                assert offer.credentials() in n.respond(client_mac)
                # Repeated identity is rejected and revokes resource authorization.
                try:
                    auth.receive(envelope(1, dict(deviceId=client_id)))
                except AuthError:
                    pass
                else:
                    raise AssertionError('repeated identity accepted')
                assert auth.session_key is None
                assert n.respond(client_mac) is None
                auth.close()
                n.close()
                results.append(dict(phase=phase, keys_match=True, stale_auth_revoked=True))
                assert identity.read_bytes() == original
            finally:
                if auth:
                    auth.close()
                if w:
                    w.close()
                if p:
                    try:
                        p.call(dict(op='close'))
                    finally:
                        p.close()
    print(json.dumps(dict(rust_private_ipc_auth='passed', cases=results,
                          identity_unchanged=True, raw_credentials_logged=False)))


def broken_ipc(binary):
    runner = json.loads(os.environ.get('CARLINK_RUST_RUNNER', '[]'))
    for header in [struct.pack('!I', 0), struct.pack('!I', 256 * 1024 + 1), b'\x00\x01']:
        parent, child = socket.socketpair()
        process = None
        try:
            process = subprocess.Popen([*runner, binary, 'hicar', 'session-worker', str(child.fileno())],
                pass_fds=(child.fileno(),), stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            child.close()
            parent.sendall(header)
            parent.shutdown(socket.SHUT_WR)
            assert process.wait(timeout=5) != 0, 'invalid private IPC accepted'
        finally:
            parent.close()
            child.close()
            if process and process.poll() is None:
                process.kill()
                process.wait(timeout=5)
    print(json.dumps(dict(rust_invalid_ipc='passed', cases=3)))


if __name__ == '__main__':
    binary = str(Path(sys.argv[1]).resolve())
    transcript(binary)
    broken_ipc(binary)
    if len(sys.argv) > 2:
        authentication(binary, str(Path(sys.argv[2]).resolve()),
                       str(Path(sys.argv[3] if len(sys.argv) > 3 else sys.argv[2]).resolve()))
