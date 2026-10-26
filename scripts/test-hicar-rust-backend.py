"""Synthetic Linux owner integration with private fake radio/session/media peers.

No Bluetooth, Wi-Fi, USB, native authentication library or identity file is used.
The real Rust owner handles TCP lifetime, fd transfer, timers and worker cleanup.
"""
import array
import hashlib
import json
import os
from pathlib import Path
import queue
import signal
import socket
import struct
import subprocess
import sys
import threading
import time


def exact(sock, count):
    data = bytearray()
    while len(data) < count:
        part = sock.recv(count - len(data))
        if not part:
            raise EOFError()
        data.extend(part)
    return bytes(data)


def free_ports():
    for _ in range(100):
        first = socket.socket()
        first.bind(('127.0.0.1', 0))
        port = first.getsockname()[1]
        sockets = [first]
        try:
            for offset in (1, 2):
                endpoint = socket.socket()
                sockets.append(endpoint)
                endpoint.bind(('127.0.0.1', port + offset))
            return port
        except OSError:
            pass
        finally:
            for endpoint in sockets:
                endpoint.close()
    raise AssertionError('no three adjacent loopback ports')


class Harness:
    address = '02:00:00:00:00:01'
    peer = '02:00:00:00:00:02'

    def __init__(self, binary):
        self.base = free_ports()
        self.sight = socket.socket()
        self.sight.bind(('127.0.0.1', 0))
        self.sight.listen()
        self.sight.settimeout(15)
        self.sight_port = self.sight.getsockname()[1]
        self.trace = []
        self.events = []
        self.errors = queue.Queue()
        self.ended = threading.Event()
        self.video = threading.Event()
        self.stopping = threading.Event()
        self.radio_lock = threading.Lock()
        self.threads = []
        self.sockets = []
        radio, child_radio = socket.socketpair(socket.AF_UNIX, socket.SOCK_SEQPACKET)
        session, child_session = socket.socketpair()
        media, child_media = socket.socketpair()
        self.radio = radio
        self.sockets.extend((radio, session, media))
        fds = (child_radio.fileno(), child_session.fileno(), child_media.fileno())
        args = [str(Path(binary).resolve()), 'hicar', 'run', '--test-mode',
                '--peer', self.peer, '--test-listen-ip', '127.0.0.1',
                '--test-base-port', str(self.base)]
        for name, fd in zip(('radio', 'session', 'media'), fds):
            args.extend((f'--test-{name}-fd', str(fd)))
        self.process = subprocess.Popen(args, pass_fds=fds, stdout=subprocess.PIPE,
                                        stderr=subprocess.PIPE, text=True)
        for child in (child_radio, child_session, child_media):
            child.close()
        self.start(self.radio_worker)
        self.start(lambda: self.rpc_worker(session, self.session))
        self.start(lambda: self.rpc_worker(media, self.media))
        self.start(self.read_events)
        self.start(self.sight_worker)

    def start(self, operation):
        def checked():
            try:
                operation()
            except (EOFError, OSError) as error:
                if not self.stopping.is_set():
                    self.errors.put(error)
            except BaseException as error:
                self.errors.put(error)
        thread = threading.Thread(target=checked, daemon=True)
        self.threads.append(thread)
        thread.start()

    def check(self):
        if not self.errors.empty():
            raise self.errors.get()

    def wait(self, predicate, seconds=5):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            self.check()
            if predicate():
                return
            if self.process.poll() is not None:
                raise AssertionError('backend exited: ' + self.process.stderr.read())
            time.sleep(.02)
        raise AssertionError('backend condition timed out; events=' + repr(self.events))

    def event(self, name):
        return [value for value in self.events if value.get('event') == name]

    def read_events(self):
        for line in self.process.stdout:
            value = json.loads(line)
            assert 'key' not in value and 'password' not in value
            self.events.append(value)

    def send_radio(self, value, fd=None):
        with self.radio_lock:
            ancillary = [] if fd is None else [(socket.SOL_SOCKET, socket.SCM_RIGHTS,
                                               array.array('i', [fd]))]
            self.radio.sendmsg([json.dumps(value).encode()], ancillary)

    def radio_worker(self):
        while True:
            value = json.loads(self.radio.recv(65536))
            operation = value.get('op', 'prepare')
            self.trace.append(('radio', operation))
            result = {'id': value['id'], 'ok': True}
            if operation == 'prepare':
                assert value['peer'] == self.peer and value['pair_seconds'] == 0
                result.update(ready=True, address=self.address, offer={
                    'ssid': 'CarLink', 'bssid': self.address, 'password': '12345678',
                    'owner_ip': '127.0.0.1', 'client_ip': '127.0.0.1',
                    'frequency': 5745, 'port': self.base})
            elif operation == 'activate':
                assert ('session', 'init') in self.trace
                # Network listeners must be bound before discoverability begins.
                for offset in range(3):
                    with socket.create_connection(('127.0.0.1', self.base + offset), 1):
                        pass
            self.send_radio(result)
            if operation == 'close':
                return

    def rpc_worker(self, endpoint, handler):
        while True:
            length, = struct.unpack('!I', exact(endpoint, 4))
            assert 0 < length <= 262144
            value = json.loads(exact(endpoint, length))
            result = dict(handler(value), ok=True)
            data = json.dumps(result).encode()
            endpoint.sendall(struct.pack('!I', len(data)) + data)
            if value.get('op') == 'close':
                return

    def session(self, value):
        operation = value.get('op', 'init')
        self.trace.append(('session', operation))
        if operation == 'init':
            seed = ('00040A00' + self.address.replace(':', '')).encode()
            own = hashlib.sha256(seed).hexdigest().upper().encode()
            return {'ready': True, 'wire_identity_sha256': hashlib.sha256(own).hexdigest()}
        if operation == 'auth-open':
            return {'generation': 1}
        if operation == 'auth':
            assert json.loads(value['envelope']) == {'synthetic': True}
            return {'metadata': {'key_ready': True}, 'key': bytes(range(16)).hex()}
        if operation == 'dmsdp-open':
            assert value['generation'] == 1
            return {'session': 2, 'initial': b'INIT'.hex()}
        if operation == 'dmsdp-receive':
            valid = bytes.fromhex(value['data']) == b'VALID'
            return {'events': [{'valid': True}] if valid else [], 'pending': [],
                    'state': {'negotiated': True, 'media_port': self.sight_port}}
        if operation == 'dmsdp-heartbeat':
            return {'heartbeat': b'H'.hex(), 'pending': []}
        if operation == 'hisight-open':
            return {'session': 3, 'initial': b'ANNOUNCE'.hex()}
        if operation == 'hisight-receive':
            return {'pending': [], 'state': {'media_endpoint': 50100,
                                             'input_endpoint': 50101}}
        if operation == 'hisight-play':
            return {'pending': [b'PLAY'.hex()]}
        if operation == 'hisight-teardown':
            return {'teardown': b'TEARDOWN'.hex()}
        return {'pending': []}

    def media(self, value):
        operation = value.get('op', 'init')
        self.trace.append(('media', operation))
        if operation == 'init':
            assert value['key'] == bytes(range(16)).hex()
            return {'ready': True, 'audio_ports': [50102, -1], 'state': {}}
        if operation == 'set-video':
            self.video.set()
        active = self.video.is_set() and not self.ended.is_set()
        return {'state': {'active': active, 'connected': active,
                          'video_ended': self.ended.is_set(), 'video_frames': int(active),
                          'epoch_video_frames': int(active)}}

    def sight_worker(self):
        endpoint, _ = self.sight.accept()
        self.sockets.append(endpoint)
        endpoint.settimeout(15)
        assert exact(endpoint, len(b'ANNOUNCE')) == b'ANNOUNCE'
        endpoint.sendall(b'SETUP')
        received = bytearray()
        while True:
            part = endpoint.recv(4096)
            if not part:
                break
            received.extend(part)
        assert b'PLAY' in received and b'TEARDOWN' in received
        self.trace.append(('sight', 'teardown-received'))

    def authenticate(self):
        self.wait(lambda: bool(self.event('ready')))
        remote, handed = socket.socketpair()
        self.send_radio({'event': 'rfcomm', 'stream': 'business', 'device': '/mock/phone',
                         'address': self.peer}, handed.fileno())
        handed.close()
        remote.sendall(b'{"synthetic":true}')
        self.wait(lambda: ('session', 'auth') in self.trace)
        remote.close()
        self.wait(lambda: ('radio', 'release') in self.trace)
        assert ('session', 'auth-close') not in self.trace

    def project(self):
        endpoint = socket.create_connection(('127.0.0.1', self.base + 2), 2)
        endpoint.settimeout(2)
        self.sockets.append(endpoint)
        assert exact(endpoint, 4) == b'INIT'
        endpoint.sendall(b'VALID')
        self.wait(lambda: any(value.get('active') for value in self.event('bridge_stats')))
        return endpoint

    def close(self):
        if self.process.poll() is None:
            self.process.send_signal(signal.SIGTERM)
            try:
                self.process.wait(timeout=6)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
                raise AssertionError('backend did not stop within six seconds')
        self.stopping.set()
        for endpoint in self.sockets + [self.sight]:
            endpoint.close()
        for thread in self.threads:
            thread.join(.3)


def run(binary):
    owner = Harness(binary)
    try:
        owner.authenticate()
        control = owner.project()
        # A second control connection must be rejected without touching the owner.
        with socket.create_connection(('127.0.0.1', owner.base + 2), 2) as duplicate:
            duplicate.settimeout(2)
            assert duplicate.recv(1) == b''
        control.sendall(b'VALID')
        time.sleep(.2)
        assert owner.trace.count(('session', 'dmsdp-open')) == 1
        assert owner.process.poll() is None
        owner.ended.set()
        owner.wait(lambda: bool(owner.event('hisight_closed')))
        assert owner.event('bridge_stats')[-1]['active'] is False
        assert owner.event('bridge_stats')[-1]['epoch_video_frames'] == 0
        owner.wait(lambda: ('sight', 'teardown-received') in owner.trace)
        assert ('session', 'dmsdp-stop-audio') in owner.trace
        assert ('media', 'close') in owner.trace
        assert ('session', 'auth-close') not in owner.trace
    finally:
        owner.close()
    owner.check()
    assert owner.process.returncode == 0
    assert ('session', 'auth-close') in owner.trace
    assert ('session', 'close') in owner.trace and ('radio', 'close') in owner.trace
    assert owner.trace.index(('radio', 'prepare')) < owner.trace.index(('session', 'init')) < owner.trace.index(('radio', 'activate'))

    owner = Harness(binary)
    try:
        owner.authenticate()
        control = owner.project()
        deadline = time.monotonic() + 10
        while not owner.event('control_peer_timeout') and time.monotonic() < deadline:
            try:
                control.sendall(b'x')  # Partial data must not refresh valid-message time.
                control.recv(4096)
            except (OSError, socket.timeout):
                pass
            time.sleep(.2)
        owner.wait(lambda: bool(owner.event('control_peer_timeout')), seconds=2)
        owner.wait(lambda: bool(owner.event('hisight_closed')))
        assert owner.trace.count(('session', 'dmsdp-receive')) >= 4
        assert owner.event('bridge_stats')[-1]['active'] is False
        assert owner.process.poll() is None
    finally:
        owner.close()
    owner.check()
    assert owner.process.returncode == 0
    print('PASS: Rust backend private workers, auth EOF, duplicate control, media EOF, valid-message deadline and bounded cleanup')


if __name__ == '__main__':
    if os.name != 'posix':
        raise SystemExit('Linux/Unix private fd integration required')
    run(sys.argv[1])
