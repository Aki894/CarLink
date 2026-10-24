#!/usr/bin/env python3
"""Bounded HiCar transport experiment with an optional native auth backend."""
import argparse
import asyncio
import ctypes
from contextlib import nullcontext
import hashlib
import json
import os
from pathlib import Path
import signal
import socket
import struct
import time

from dbus_next import BusType, DBusError, MessageType, Variant
from dbus_next.aio import MessageBus
from dbus_next.constants import PropertyAccess
from dbus_next.service import ServiceInterface, dbus_property, method
from nearby_wire import Decoder, ProtocolError, pack_frame, read_tlvs
from control import FE36, Negotiator
from auth_wire import AuthError, AuthSession, JsonStream
from identity import DEFAULT_PATH, load_or_create
from dmsdp_wire import Decoder as DmsdpDecoder, Negotiation as DmsdpNegotiation
from control_wire import Session as ControlSession
from hisight import receive as receive_hisight
from audio_transport import Receiver as AudioReceiver, synchronize

FDEE = '0000fdee-0000-1000-8000-00805f9b34fb'
FE35 = '0000fe35-0000-1000-8000-00805f9b34fb'
ROOT = '/org/carlink/hicar_probe'
LIMIT = 1024 * 1024


def mgmt_command(index, opcode, payload=b''):
    # Linux Bluetooth management control channel, independent of BlueZ's D-Bus.
    with socket.socket(socket.AF_BLUETOOTH, socket.SOCK_RAW, socket.BTPROTO_HCI) as ctl:
        # Python 3.13's HCI bind() only supports the legacy raw-channel tuple.
        address = ctypes.create_string_buffer(struct.pack('@HHH', socket.AF_BLUETOOTH,
                                                          0xffff, 3))
        libc = ctypes.CDLL(None, use_errno=True)
        libc.bind.argtypes = (ctypes.c_int, ctypes.c_void_p, ctypes.c_uint)
        libc.bind.restype = ctypes.c_int
        if libc.bind(ctl.fileno(), address, 6) != 0:
            error = ctypes.get_errno()
            raise OSError(error, os.strerror(error))
        ctl.settimeout(3)
        ctl.sendall(struct.pack('<HHH', opcode, index, len(payload)) + payload)
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            ctl.settimeout(max(0.001, deadline - time.monotonic()))
            packet = ctl.recv(4096)
            if len(packet) < 9:
                continue
            event, device, length = struct.unpack_from('<HHH', packet)
            if len(packet) != length + 6 or device != index or event not in (1, 2):
                continue
            command, status = struct.unpack_from('<HB', packet, 6)
            if command != opcode:
                continue
            if status:
                raise RuntimeError(f'Bluetooth management command {opcode} failed: {status}')
            if event == 1:
                return packet[9:]
        raise TimeoutError('Bluetooth management reply timeout')


def mgmt_settings(index):
    data = mgmt_command(index, 4)  # Read Controller Information
    if len(data) < 17:
        raise RuntimeError('truncated Bluetooth controller information')
    return struct.unpack_from('<I', data, 13)[0]


def set_setting(index, opcode, mask, enabled):
    data = mgmt_command(index, opcode, bytes([bool(enabled)]))
    if len(data) < 4 or bool(struct.unpack_from('<I', data)[0] & mask) != enabled:
        raise RuntimeError('controller did not apply requested setting')
    log('controller_setting', opcode=opcode, enabled=enabled)


def service_record(channel, service=FE35):
    if not 1 <= channel <= 30:
        raise ValueError('RFCOMM channel must be between 1 and 30')
    return f'''<record>
      <attribute id="0x0001"><sequence><uuid value="{service}"/></sequence></attribute>
      <attribute id="0x0004"><sequence>
        <sequence><uuid value="0x0100"/></sequence>
        <sequence><uuid value="0x0003"/><uint8 value="{channel}"/></sequence>
      </sequence></attribute>
      <attribute id="0x0005"><sequence><uuid value="0x1002"/></sequence></attribute>
      <attribute id="0x0100"><text value="NearbyServerSocket"/></attribute>
    </record>'''


def beacon(model, sequence=1):
    if len(model) != 8:
        raise ValueError('model must contain exactly eight hex characters')
    raw = bytes.fromhex(model)
    if len(raw) != 4:
        raise ValueError('model must contain exactly four bytes')
    if not 0 <= sequence <= 255:
        raise ValueError('sequence must be between 0 and 255')
    return bytes([1, 1, 1, 3]) + raw[:3] + bytes([4, raw[3], 16, sequence])


def log(event, **fields):
    print(json.dumps(dict(event=event, monotonic=round(time.monotonic(), 3), **fields)),
          flush=True)


async def observe(sock, session, capture_dir=None, limit=LIMIT, negotiator=None,
                  decode_control=True, auth_session=None, resource=None, dmsdp=None, control=None,
                  live_socket=None, audio_binary=None):
    """Observe a byte stream without treating recv boundaries as protocol frames."""
    total = 0
    digest = hashlib.sha256()
    capture = None
    plaintext_capture = None
    media_task = None
    clock_task = None
    audio_receiver = None
    audio_tasks = []
    heartbeat_task = None
    sink = None
    control_lock = asyncio.Lock()

    async def send_control(records):
        async with control_lock:
            for record in records:
                await asyncio.wait_for(asyncio.get_running_loop().sock_sendall(sock, record), 2)
    decoder = Decoder()
    parsing = decode_control
    auth_stream = JsonStream() if auth_session else None
    dmsdp_stream = DmsdpDecoder() if dmsdp else None
    try:
        if capture_dir:
            path = Path(capture_dir) / f'session-{time.time_ns()}-{session}.bin'
            capture = os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL,
                                       0o600), 'wb')
            if control:
                plaintext_capture = os.fdopen(os.open(str(path) + '.plain',
                    os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'wb')
                def save_plaintext(payload):
                    # Length-delimited private records, bounded by the same
                    # receive limit. Never store the session key itself.
                    plaintext_capture.write(len(payload).to_bytes(2, 'big') + payload)
                    plaintext_capture.flush()
                control.on_plaintext = save_plaintext
        sock.setblocking(False)
        loop = asyncio.get_running_loop()
        if control and live_socket:
            from bridge import Bridge
            sink = Bridge(live_socket, audio_binary, log)
        if control and control.speaker:
            audio_receiver = AudioReceiver(control.device_info['localIp'],
                                           control.device_info['remoteIp'],
                                           bytes(control.cipher.key), capture_dir, log, sink)
            control.speaker.ports = audio_receiver.ports
            audio_tasks = [asyncio.create_task(audio_receiver.receive(0))]
        if control:
            await asyncio.wait_for(loop.sock_sendall(sock, control.initial()), 2)
            log('control_options_sent', session=session, crypto_ability=222)
        while total < limit:
            data = await loop.sock_recv(sock, min(4096, limit - total))
            if not data:
                break
            total += len(data)
            digest.update(data)
            if capture:
                capture.write(data)
                capture.flush()
            log('rx_chunk', session=session, size=len(data), total=total)
            if control:
                try:
                    for event in control.receive(data):
                        log('control_message', session=session, **event)
                        if audio_receiver:
                            if event.get('audio_setup_sent'):
                                audio_receiver.begin()
                            elif event.get('audio_ready'):
                                audio_receiver.active = True
                            elif event.get('audio_stopped'):
                                audio_receiver.active = False
                                if sink:
                                    sink.audio_stop()
                    await send_control(control.take_pending())
                    if control.negotiated and heartbeat_task is None:
                        async def heartbeat():
                            try:
                                while True:
                                    await send_control([control.heartbeat()])
                                    await asyncio.sleep(1)
                            except (OSError, asyncio.TimeoutError) as error:
                                log('control_heartbeat_stopped', error=type(error).__name__)
                                try:
                                    sock.shutdown(socket.SHUT_RDWR)
                                except OSError:
                                    pass
                        heartbeat_task = asyncio.create_task(heartbeat())
                    if control.clock and clock_task is None:
                        clock_task = asyncio.create_task(synchronize(control.device_info['localIp'],
                            control.device_info['remoteIp'], control.clock[2],
                            bytes(control.cipher.key), log,
                            audio_receiver.clock if audio_receiver else None))
                    if control.media_port and media_task is None:
                        async def observe_media():
                            media = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
                            media.setblocking(False)
                            try:
                                media.bind((control.device_info['localIp'], 0))
                                await asyncio.wait_for(loop.sock_connect(media,
                                    (control.device_info['remoteIp'], control.media_port)), 5)
                                log('hisight_connected', port=control.media_port)
                                await receive_hisight(media, bytes(control.cipher.key), capture_dir, log, sink)
                            except (OSError, asyncio.TimeoutError) as error:
                                log('hisight_error', error=type(error).__name__)
                            finally:
                                media.close()
                        media_task = asyncio.create_task(observe_media())
                except ProtocolError as error:
                    log('control_parse_stopped', session=session, reason=str(error))
                    return
            if dmsdp_stream:
                try:
                    for packet in dmsdp_stream.feed(data):
                        log('dmsdp_packet', session=session, module=packet.module, size=len(packet.payload))
                        reply = dmsdp.respond(packet)
                        await asyncio.wait_for(loop.sock_sendall(sock, reply), 2)
                        log('dmsdp_control_port_offered', session=session, port=dmsdp.port)
                except ProtocolError as error:
                    log('dmsdp_parse_stopped', session=session, reason=str(error))
                    return
            if auth_stream:
                try:
                    for envelope in auth_stream.feed(data):
                        reply, metadata = auth_session.receive(envelope)
                        log('hicar_envelope', session=session, **metadata)
                        if (resource and metadata.get('backend') == 'hichain'
                                and 0 in metadata.get('results', ()) and metadata.get('key_ready')):
                            resource.authenticated = True
                            resource.session_key = auth_session.backend.session_key
                        if reply:
                            await asyncio.wait_for(loop.sock_sendall(sock, reply), 2)
                            log('authentication_reply', session=session, size=len(reply))
                except AuthError as error:
                    log('auth_parse_stopped', session=session, reason=str(error))
                    return  # Close a failed authentication session; do not leave a live socket.
            if parsing:
                try:
                    for frame in decoder.feed(data):
                        fields = dict(session=session, kind=frame.kind, flags=frame.flags,
                                      encrypted=frame.encrypted, size=len(frame.payload))
                        if frame.kind in (0, 2, 4, 33) and not frame.encrypted:
                            fields['tlvs'] = [dict(tag=tag, size=len(value))
                                              for tag, value in read_tlvs(frame.payload)]
                        log('nearby_frame', **fields)
                        if negotiator:
                            response = negotiator.respond(frame)
                            if response:
                                await asyncio.wait_for(loop.sock_sendall(sock, pack_frame(frame.kind, response)), 2)
                                event = {0: 'ccmp_response', 2: 'business_socket_offered',
                                         4: 'resource_response', 33: 'network_command_response'}[frame.kind]
                                log(event, session=session,
                                    complete=negotiator.complete, framing_encryption=False)
                            if negotiator.resource:
                                for kind, payload in negotiator.resource.take_pending():
                                    await asyncio.wait_for(loop.sock_sendall(sock, pack_frame(kind, payload)), 2)
                                    log('network_followup', session=session, kind=kind)
                except ProtocolError as error:
                    log('parse_stopped', session=session, reason=str(error))
                    parsing = False  # Keep private capture for unsupported formats.
        if total == limit:
            log('capture_limit', session=session, limit=limit)
    finally:
        if heartbeat_task:
            heartbeat_task.cancel()
            await asyncio.gather(heartbeat_task, return_exceptions=True)
        if control and control.speaker:
            control.stop_audio()
            try:
                await asyncio.wait_for(send_control(control.take_pending()), 1)
                log('audio_teardown_sent')
            except (OSError, asyncio.TimeoutError):
                log('audio_teardown_stopped')
        if media_task:
            media_task.cancel()
            await asyncio.gather(media_task, return_exceptions=True)
        for task in [clock_task, *audio_tasks]:
            if task:
                task.cancel()
        await asyncio.gather(*(t for t in [clock_task, *audio_tasks] if t), return_exceptions=True)
        if audio_receiver:
            audio_receiver.close()
        if sink:
            sink.close()
        if plaintext_capture:
            plaintext_capture.close()
        if control:
            control.close()
        if dmsdp_stream:
            try:
                dmsdp_stream.finish()
            except ProtocolError as error:
                log('incomplete_dmsdp_stream', session=session, reason=str(error))
        if auth_session:
            if resource:
                resource.authenticated = False
                resource.session_key = None
            auth_session.close()
        if auth_stream:
            try:
                auth_stream.finish()
            except AuthError as error:
                log('incomplete_auth_stream', session=session, reason=str(error))
        if parsing:
            try:
                decoder.finish()
            except ProtocolError as error:
                log('incomplete_stream', session=session, reason=str(error))
        if capture:
            capture.close()
        sock.close()
        log('stream_closed', session=session, bytes=total, sha256=digest.hexdigest())


class Advertisement(ServiceInterface):
    def __init__(self, payload, stop, tx_power=0):
        super().__init__('org.bluez.LEAdvertisement1')
        self.payload, self.stop = payload, stop
        self.instance = 0
        self.tx_power = tx_power

    @dbus_property(access=PropertyAccess.READ)
    def Type(self) -> 's':
        return 'peripheral'

    @dbus_property(access=PropertyAccess.READ)
    def Discoverable(self) -> 'b':
        return True

    @dbus_property(access=PropertyAccess.READ)
    def LocalName(self) -> 's':
        return 'CarLink'

    @dbus_property()
    def TxPower(self) -> 'n':
        # Supported by the tested AIC8800 adapter (-9..11 dBm).
        return self.tx_power

    @TxPower.setter
    def TxPower(self, value: 'n'):
        # Board BlueZ reports the controller-selected power back through Set.
        self.tx_power = value

    @dbus_property()
    def Instance(self) -> 'y':
        return self.instance

    @Instance.setter
    def Instance(self, value: 'y'):
        self.instance = value

    @dbus_property(access=PropertyAccess.READ)
    def ServiceData(self) -> 'a{sv}':
        return {FDEE: Variant('ay', self.payload)}

    @method()
    def Release(self):
        log('advertisement_released')
        self.stop.set()


class PeerAgent(ServiceInterface):
    def __init__(self, peer_path):
        super().__init__('org.bluez.Agent1')
        self.peer_path = peer_path

    def check(self, device):
        if device != self.peer_path:
            raise DBusError('org.bluez.Error.Rejected', 'not the selected test phone')

    @method()
    def Release(self):
        log('agent_released')

    @method()
    def Cancel(self):
        log('agent_cancelled')

    @method()
    def RequestConfirmation(self, device: 'o', passkey: 'u'):
        self.check(device)
        log('ssp_confirmation')  # Never log the passkey.

    @method()
    def RequestAuthorization(self, device: 'o'):
        self.check(device)

    @method()
    def AuthorizeService(self, device: 'o', uuid: 's'):
        self.check(device)
        if uuid.lower() not in (FE35, FE36):
            raise DBusError('org.bluez.Error.Rejected', 'unsupported probe service')

    @method()
    def RequestPinCode(self, device: 'o') -> 's':
        raise DBusError('org.bluez.Error.Rejected', 'legacy PIN unsupported')

    @method()
    def RequestPasskey(self, device: 'o') -> 'u':
        raise DBusError('org.bluez.Error.Rejected', 'passkey entry unsupported')


class Profile(ServiceInterface):
    def __init__(self, stop, capture_dir, peer_path=None, control_summary=None,
                 decode_control=True):
        super().__init__('org.bluez.Profile1')
        self.stop, self.capture_dir = stop, capture_dir
        self.tasks = {}
        self.sockets = {}
        self.sequence = 0
        self.peer_path = peer_path
        self.control_summary = control_summary
        self.decode_control = decode_control
        self.business_channel = None
        self.auth_identity = None
        self.auth_model = None
        self.resource = None

    @method()
    def Release(self):
        log('profile_released')
        self.stop.set()

    @method()
    def NewConnection(self, device: 'o', fd: 'h', properties: 'a{sv}'):
        if (self.stop.is_set() or len(self.tasks) >= 4 or device in self.tasks
                or (self.peer_path and device != self.peer_path)):
            os.close(fd)
            raise DBusError('org.bluez.Error.Rejected', 'probe connection limit')
        try:
            sock = socket.socket(fileno=fd)
        except Exception:
            os.close(fd)
            raise
        self.sequence += 1
        session = self.sequence
        negotiator = Negotiator(self.control_summary, self.business_channel,
                                 self.resource) if self.control_summary else None
        auth_session = AuthSession(self.auth_identity, self.auth_model,
                                   getattr(self, 'backend_factory', None)) if self.auth_identity else None
        task = asyncio.create_task(observe(sock, session, self.capture_dir,
                                           negotiator=negotiator, decode_control=self.decode_control,
                                           auth_session=auth_session, resource=self.resource))
        self.tasks[device] = task
        self.sockets[device] = sock

        def done(completed):
            self.tasks.pop(device, None)
            self.sockets.pop(device, None)
            sock.close()  # Also covers cancellation before observe() starts.
            if not completed.cancelled() and completed.exception():
                log('stream_error', session=session,
                    error=type(completed.exception()).__name__)
        task.add_done_callback(done)
        log('rfcomm_connected', session=session,
            stream='control' if self.decode_control else 'business')

    @method()
    def RequestDisconnection(self, device: 'o'):
        task = self.tasks.get(device)
        if task:
            task.cancel()
            self.sockets[device].close()

    async def close(self):
        tasks = list(self.tasks.values())
        for task in tasks:
            task.cancel()
        for sock in list(self.sockets.values()):
            sock.close()
        await asyncio.gather(*tasks, return_exceptions=True)


async def close_tcp_streams(controls, clients, sockets):
    # Control observers own the HiSight tasks. Let those send TEARDOWN before
    # closing Nearby TCP, RFCOMM authentication, or the temporary hotspot.
    first = list(controls)
    for task in first:
        task.cancel()
    await asyncio.gather(*first, return_exceptions=True)
    rest = [task for task in list(clients) if task not in first]
    for task in rest:
        task.cancel()
    await asyncio.gather(*rest, return_exceptions=True)
    for client in list(sockets):
        client.close()


async def run(args):
    payload = beacon(args.model, args.sequence)
    stop = asyncio.Event()
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGINT, signal.SIGTERM):
        loop.add_signal_handler(sig, stop.set)
    bus = await MessageBus(bus_type=BusType.SYSTEM, negotiate_unix_fd=True).connect()

    def property_trace(message):
        if (message.message_type == MessageType.METHOD_CALL
                and message.path == ROOT + '/advertisement'
                and message.interface == 'org.freedesktop.DBus.Properties'
                and message.member == 'Set' and len(message.body) >= 2):
            log('advertisement_property_set', property=message.body[1])
        return False
    bus.add_message_handler(property_trace)
    peer_path = args.adapter + '/dev_' + args.peer.upper().replace(':', '_') if args.peer else None
    profile = Profile(stop, args.capture_dir, peer_path)
    business_profile = Profile(stop, args.capture_dir, peer_path, decode_control=False)
    if args.network_offer:
        from network_wire import Resource
        profile.resource = business_profile.resource = Resource(args.network_offer)
    if args.reply_identity:
        business_profile.auth_identity = load_or_create(args.identity)
        business_profile.auth_model = args.model.upper()
        if args.auth_library:
            from hichain import Runtime, Session
            runtime = Runtime(args.auth_library, args.auth_store,
                              business_profile.auth_identity.device_id)
            business_profile.backend_factory = lambda own, peer, pin: Session(
                runtime, own, peer, pin, package='com.huawei.hicar', service='CarDevice')
    bus.export(ROOT + '/profile', profile)
    bus.export(ROOT + '/business', business_profile)
    bus.export(ROOT + '/advertisement', Advertisement(payload, stop, args.tx_power))
    pm = am = agent_manager = None
    agent_registered = False
    profile_registered = advertisement_registered = False
    business_registered = False
    restore_connectable = False
    restore_bondable = False
    tcp_listener = tcp_task = None
    data_listener = data_task = None
    control_listener = control_task = None
    tcp_clients = set()
    tcp_controls = set()
    tcp_sockets = set()
    controller_index = int(args.adapter.rsplit('hci', 1)[1])
    try:
        intro = await bus.introspect('org.bluez', '/org/bluez')
        pm = bus.get_proxy_object('org.bluez', '/org/bluez', intro).get_interface(
            'org.bluez.ProfileManager1')
        intro = await bus.introspect('org.bluez', args.adapter)
        adapter = bus.get_proxy_object('org.bluez', args.adapter, intro)
        props = adapter.get_interface('org.freedesktop.DBus.Properties')
        if not (await props.call_get('org.bluez.Adapter1', 'Powered')).value:
            raise RuntimeError('Bluetooth adapter is powered off')
        if args.reply_control:
            address = (await props.call_get('org.bluez.Adapter1', 'Address')).value
            own_bluetooth_address = address
            seed = args.model.upper() + address.replace(':', '').upper()
            profile.control_summary = hashlib.sha256(seed.encode('ascii')).hexdigest().upper().encode('ascii')
            if args.reply_identity:
                business_profile.auth_identity = business_profile.auth_identity.for_adapter(args.model, address)
                # Discovery summary, business identity and native signing IDs
                # must agree. The Runtime above keeps its persistent HUKS seed.
                assert business_profile.auth_identity.device_id.encode('ascii') == profile.control_summary
        if args.network_offer:
            tcp_listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            tcp_listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            tcp_listener.bind((args.network_offer.owner_ip, args.network_offer.port))
            tcp_listener.listen(4)
            tcp_listener.setblocking(False)

            async def accept_tcp(listener, port, decode_control, dmsdp_port=None, control=False):
                sequence = 0
                while True:
                    client, address = await loop.sock_accept(listener)
                    if (address[0] != args.network_offer.client_ip
                            or not profile.resource.authenticated or len(tcp_clients) >= 4):
                        client.close()
                        log('tcp_rejected', reason='unselected_or_unauthenticated_peer')
                        continue
                    sequence += 1
                    session = f'tcp-{port}-{sequence}'
                    log('tcp_connected', session=session, port=port)
                    negotiator = Negotiator(profile.control_summary, resource=profile.resource) if decode_control else None
                    task = asyncio.create_task(observe(client, session, args.capture_dir,
                                                       negotiator=negotiator, decode_control=decode_control,
                                                       dmsdp=DmsdpNegotiation(dmsdp_port) if dmsdp_port else None,
                                                       control=ControlSession(profile.resource.session_key,
                                                           dict(deviceId=business_profile.auth_identity.device_id,
                                                                deviceName=business_profile.auth_identity.name,
                                                                deviceType=4, bluetoothMac=own_bluetooth_address,
                                                                btName='CarLink', localIp=args.network_offer.owner_ip,
                                                                remoteIp=args.network_offer.client_ip, port=0,
                                                                properties={'2003': args.model, '5004': True,
                                                                            '5014': False}), audio=args.audio)
                                                       if control else None,
                                                       live_socket=args.live_socket,
                                                       audio_binary=args.audio_binary))
                    tcp_clients.add(task)
                    if control:
                        tcp_controls.add(task)
                    tcp_sockets.add(client)

                    def completed(task, client=client):
                        tcp_clients.discard(task)
                        tcp_controls.discard(task)
                        tcp_sockets.discard(client)
                        client.close()
                        if not task.cancelled() and task.exception():
                            log('tcp_stream_error', error=type(task.exception()).__name__)
                    task.add_done_callback(completed)
            data_listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            data_listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            data_listener.bind((args.network_offer.owner_ip, args.network_offer.port + 1))
            data_listener.listen(4)
            data_listener.setblocking(False)
            profile.resource.data_port = args.network_offer.port + 1
            control_listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            control_listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            control_listener.bind((args.network_offer.owner_ip, args.network_offer.port + 2))
            control_listener.listen(4)
            control_listener.setblocking(False)
            tcp_task = asyncio.create_task(accept_tcp(tcp_listener, args.network_offer.port, True))
            data_task = asyncio.create_task(accept_tcp(data_listener, profile.resource.data_port, False,
                                                       args.network_offer.port + 2))
            control_task = asyncio.create_task(accept_tcp(control_listener, args.network_offer.port + 2,
                                                         False, control=args.dmsdp_options))
            log('network_listener_ready', port=args.network_offer.port, frequency=args.network_offer.frequency)
        if args.allow_incoming:
            settings = mgmt_settings(controller_index)
            restore_connectable = not bool(settings & 2)
            restore_bondable = not bool(settings & 16)
            bus.export(ROOT + '/agent', PeerAgent(peer_path))
            intro = await bus.introspect('org.bluez', '/org/bluez')
            agent_manager = bus.get_proxy_object('org.bluez', '/org/bluez', intro).get_interface(
                'org.bluez.AgentManager1')
            await agent_manager.call_register_agent(ROOT + '/agent', 'NoInputNoOutput')
            agent_registered = True
            await agent_manager.call_request_default_agent(ROOT + '/agent')
            if restore_bondable:
                set_setting(controller_index, 9, 16, True)
            if restore_connectable:
                set_setting(controller_index, 7, 2, True)
        # Insecure RFCOMM matches the reference transport; HiChain must authenticate
        # the application later. This probe never accepts or fabricates auth results.
        if args.reply_control:
            await pm.call_register_profile(ROOT + '/business', FE36, {
                'Name': Variant('s', 'NearbyBusinessSocket'),
                'Role': Variant('s', 'server'),
                'Channel': Variant('q', args.channel + 1),
                'ServiceRecord': Variant('s', service_record(args.channel + 1, FE36)),
                'RequireAuthentication': Variant('b', False),
                'RequireAuthorization': Variant('b', False),
            })
            business_registered = True
            profile.business_channel = args.channel + 1
        await pm.call_register_profile(ROOT + '/profile', FE35, {
            'Name': Variant('s', 'NearbyServerSocket'),
            'Role': Variant('s', 'server'),
            'Channel': Variant('q', args.channel),
            'ServiceRecord': Variant('s', service_record(args.channel)),
            'RequireAuthentication': Variant('b', False),
            'RequireAuthorization': Variant('b', False),
        })
        profile_registered = True
        am = adapter.get_interface('org.bluez.LEAdvertisingManager1')
        await am.call_register_advertisement(ROOT + '/advertisement', {})
        advertisement_registered = True
        log('ready', duration=args.duration, transport='FE35', channel=args.channel,
            hichain_authentication=bool(args.auth_library))
        waiter = asyncio.create_task(stop.wait())
        disconnected = asyncio.create_task(bus.wait_for_disconnect())
        try:
            done, _ = await asyncio.wait([waiter, disconnected], timeout=args.duration,
                                         return_when=asyncio.FIRST_COMPLETED)
            if disconnected in done:
                raise RuntimeError('system bus disconnected')
        finally:
            for task in (waiter, disconnected):
                if not task.done():
                    task.cancel()
            await asyncio.gather(waiter, disconnected, return_exceptions=True)
    finally:
        stop.set()
        for task in (tcp_task, data_task, control_task):
            if task:
                task.cancel()
        await asyncio.gather(*(task for task in (tcp_task, data_task, control_task) if task),
                             return_exceptions=True)
        await close_tcp_streams(tcp_controls, tcp_clients, tcp_sockets)
        await profile.close()
        await business_profile.close()
        # Disconnecting the bus also removes all owned BlueZ registrations.
        for registered, manager, member, path in (
            (advertisement_registered, am, 'call_unregister_advertisement', ROOT + '/advertisement'),
            (profile_registered, pm, 'call_unregister_profile', ROOT + '/profile'),
            (business_registered, pm, 'call_unregister_profile', ROOT + '/business'),
        ):
            if registered:
                try:
                    await asyncio.wait_for(getattr(manager, member)(path), 3)
                except Exception as error:
                    log('unregister_error', error=type(error).__name__)
        if tcp_listener:
            tcp_listener.close()
        if data_listener:
            data_listener.close()
        if control_listener:
            control_listener.close()
        if agent_registered:
            try:
                await asyncio.wait_for(agent_manager.call_unregister_agent(ROOT + '/agent'), 3)
            except Exception as error:
                log('agent_unregister_error', error=type(error).__name__)
        bus.disconnect()
        try:
            if restore_bondable:
                set_setting(controller_index, 9, 16, False)
        finally:
            if restore_connectable:
                set_setting(controller_index, 7, 2, False)
        log('stopped')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--model', required=True, help='local reference model, eight hex characters')
    parser.add_argument('--duration', type=int, default=60, help='1..900 seconds')
    parser.add_argument('--adapter', default='/org/bluez/hci0')
    parser.add_argument('--channel', type=int, default=23, help='RFCOMM channel, 1..30')
    parser.add_argument('--sequence', type=int, default=1, help='discovery sequence, 0..255')
    parser.add_argument('--tx-power', type=int, default=0, help='requested advertising power in dBm')
    parser.add_argument('--allow-incoming', action='store_true',
                        help='temporarily enable BR/EDR incoming connections; restore on exit')
    parser.add_argument('--peer', help='selected phone Bluetooth address; required for --allow-incoming')
    parser.add_argument('--reply-control', action='store_true',
                        help='reply to channel negotiation and offer the FE36 HiCar business socket')
    parser.add_argument('--reply-identity', action='store_true', help='reply to the HiCar identity handshake')
    parser.add_argument('--identity', default=DEFAULT_PATH, help='private persistent identity/PIN file')
    parser.add_argument('--auth-library', help='optional pinned Linux HiChain shared library')
    parser.add_argument('--auth-store', default='/var/lib/carlink/hicar/keys',
                        help='private persistent HUKS directory')
    parser.add_argument('--network-interface', help='temporarily replace CarPlay hotspot with a HiCar AP')
    parser.add_argument('--dmsdp-options', action='store_true',
                        help='negotiate DMSDP AES control, symmetric heartbeats and clock sync')
    parser.add_argument('--audio', action='store_true', help='enable experimental speaker service')
    parser.add_argument('--live-socket', help='experimental CarLife output socket (private absolute path)')
    parser.add_argument('--carlife-binary', default='/usr/local/bin/carlink')
    parser.add_argument('--audio-binary', default='/usr/local/lib/carlink/carplay/bin/carlink-audio')
    parser.add_argument('--capture-dir', help='optional private directory for raw streams')
    args = parser.parse_args()
    if args.live_socket and not (args.audio and args.network_interface and args.dmsdp_options):
        parser.error('--live-socket requires --audio, --network-interface and --dmsdp-options')
    if not 1 <= args.duration <= 900:
        parser.error('duration must be between 1 and 900 seconds')
    if not (args.adapter.startswith('/org/bluez/hci')
            and args.adapter[len('/org/bluez/hci'):].isdigit()):
        parser.error('adapter must be /org/bluez/hci followed by its numeric index')
    if args.allow_incoming and not args.peer:
        parser.error('--allow-incoming requires --peer')
    if args.reply_identity and not (args.reply_control and args.peer):
        parser.error('--reply-identity requires --reply-control and --peer')
    if args.auth_library and not args.reply_identity:
        parser.error('--auth-library requires --reply-identity')
    if args.network_interface and not args.auth_library:
        parser.error('--network-interface requires --auth-library')
    if args.dmsdp_options and not args.network_interface:
        parser.error('--dmsdp-options requires --network-interface')
    if args.audio and not args.dmsdp_options:
        parser.error('--audio requires --dmsdp-options')
    if args.peer:
        parts = args.peer.split(':')
        if len(parts) != 6 or any(len(part) != 2 for part in parts):
            parser.error('peer must be a six-byte colon-separated Bluetooth address')
        try:
            bytes.fromhex(''.join(parts))
        except ValueError:
            parser.error('peer must contain hexadecimal bytes')
    try:
        beacon(args.model, args.sequence)
        service_record(args.channel)
        if args.reply_control:
            service_record(args.channel + 1, FE36)
        if not -127 <= args.tx_power <= 20:
            raise ValueError('tx-power must be between -127 and 20 dBm')
    except ValueError as error:
        parser.error(str(error))
    if args.capture_dir:
        Path(args.capture_dir).mkdir(mode=0o700, parents=True, exist_ok=True)
    import fcntl  # Linux-only runtime; pure helpers can be tested on Windows.
    lock = open('/run/lock/carlink-hicar-probe.lock', 'a')
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        parser.error('another HiCar probe is already running')
    try:
        if args.network_interface:
            from network import Hotspot
            context = Hotspot(args.network_interface)
        else:
            context = nullcontext(None)
        with context as args.network_offer:
            if args.live_socket:
                from bridge import CarLife
                output = CarLife(args.carlife_binary, args.live_socket)
            else:
                output = nullcontext()
            with output:
                asyncio.run(asyncio.wait_for(run(args), timeout=args.duration + 30))
    finally:
        lock.close()


if __name__ == '__main__':
    main()
