import json
import asyncio
import socket
import unittest
from cipher import Cipher
from control_wire import Decoder, Messages, Session as ControlSession, pack_frame
from hisight import Session, message, graceful_close
from hisight_media import Depacketizer
from hisight_input import Input
from nearby_wire import ProtocolError, pack_uint, read_uint


class HiSightTests(unittest.TestCase):
    def negotiate(self):
        session = Session(bytes(range(16)))
        peer = Cipher(bytes(range(16)), 65001)
        announce = (b'ANNOUNCE * RTSP/1.0\r\nCSeq: 1\r\n\r\n'
                    b'encrypt_description: encrypt_list=aes128ctr;version=1\r\n')
        session.receive(len(announce).to_bytes(4, 'big') + announce)
        session.pending.clear()
        return session, peer

    def test_record_fragmentation_and_missing_content_length(self):
        announce = (b'ANNOUNCE * RTSP/1.0\r\nCSeq: 1\r\n\r\n'
                    b'encrypt_description: encrypt_list=aes128ctr;version=1\r\n')
        wire = len(announce).to_bytes(4, 'big') + announce
        for split in range(len(wire) + 1):
            session = Session(bytes(range(16)))
            first, _ = session.receive(wire[:split])
            second, _ = session.receive(wire[split:])
            self.assertTrue(session.encrypted)
            self.assertIn(dict(hisight_encryption=True), first + second)
            reply = session.pending[0]
            self.assertTrue(reply[4:].startswith(b'RTSP/1.0 200'))
            session.cipher.close()

    def test_capabilities_and_setup_endpoint(self):
        session, peer = self.negotiate()
        def deliver(payload):
            data = peer.encrypt(payload)
            return session.receive(len(data).to_bytes(4, 'big') + data)[0]
        deliver(message('GET_PARAMETER * RTSP/1.0', 2,
                        b'his_video_formats\r\nhis_vtp\r\nhis_uibc_capability\r\n'))
        reply = Messages('RTSP').feed(peer.decrypt(session.pending.pop()[4:]))[0]
        self.assertIn(b'width 800;height 480', reply.body)
        self.assertNotIn(b'his_vtp', reply.body)
        self.assertIn(b'input_category_list=GENERIC;', reply.body)
        deliver(message('SET_PARAMETER * RTSP/1.0', 3,
            b'his_presentation_url: rtsp://192.168.49.2/hisight/streamid=0 none\r\n'
            b'his_trigger_method: SETUP\r\n'))
        self.assertEqual(session.setup_sequence, 3)
        deliver(message('RTSP/1.0 200 OK', 3, headers={
            'Transport': 'RTP/AVP/UDP;unicast;server_port=49000;remotectl_port=49001'}))
        self.assertEqual(session.media_endpoint, 49000)
        self.assertEqual(session.input_endpoint, 49001)
        # The real source counts Transport as content but emits it in the
        # header block. Its outer record still gives an exact bounded length.
        deliver(b'RTSP/1.0 200 OK\r\nCSeq: 3\r\nContent-Length: 60\r\n'
                b'Transport: RTP/AVP/UDP;server_port=49000;\r\n')
        self.assertEqual(session.media_endpoint, 49000)
        session.cipher.close(); peer.close()

    def test_encrypted_input_and_coordinate_bounds(self):
        key = bytes(range(16))
        sender, peer = Input(key), Cipher(key)
        try:
            packet = sender.key('enter', True)
            self.assertEqual(int.from_bytes(packet[2:4], 'big'), len(packet))
            plain = peer.decrypt(packet[4:])
            self.assertEqual(plain[:6], bytes([255]) * 6)
            self.assertEqual(plain[6:10], b'\x03\x00\x09\x00')
            self.assertEqual(int.from_bytes(plain[10:12], 'big'), 23)
            touch = peer.decrypt(sender.touch(799, 479, True)[4:])
            self.assertEqual(touch[6:15], b'\x00\x00\x0b\x01\x00\x03\x1f\x01\xdf')
            for x, y in ((800, 0), (-1, 0), (0, 480), (True, 0)):
                with self.assertRaises(ProtocolError):
                    sender.touch(x, y, True)
            with self.assertRaises(ProtocolError):
                sender.key('unknown', True)
        finally:
            sender.cipher.close(); peer.close()

    def test_invalid_frame_and_unoffered_cipher(self):
        for wire in (bytes(4), (65001).to_bytes(4, 'big')):
            session = Session(bytes(16))
            with self.assertRaises(ProtocolError):
                session.receive(wire)
            session.cipher.close()
        session = Session(bytes(16))
        payload = message('ANNOUNCE * RTSP/1.0', 1, b'encrypt_description: encrypt_list=aes128gcm;\r\n')
        with self.assertRaises(ProtocolError):
            session.receive(len(payload).to_bytes(4, 'big') + payload)
        session.cipher.close()

    def test_rtp_pes_fragment_assembly_decryption_and_loss(self):
        key = bytes(range(16)); peer = Cipher(key, 2 * 1024 * 1024)
        receiver = Depacketizer(key)
        es = b'\0\0\0\1\x67example-sps'
        encrypted = peer.encrypt(es)
        pes = b'\0\0\1\xe0' + (len(encrypted) + 8).to_bytes(2, 'big')
        pes += b'\x84\x80\x05' + bytes(5) + encrypted
        def packet(seq, marker, body):
            return bytes([0x80, 96 | (128 if marker else 0)]) + seq.to_bytes(2, 'big') + bytes(8) + body
        self.assertIsNone(receiver.receive(packet(0xffff, False, pes[:18])))
        self.assertEqual(receiver.receive(packet(0, True, pes[18:]))[2], es)
        receiver.receive(packet(1, False, pes[:18]))
        with self.assertRaises(ProtocolError):
            receiver.receive(packet(3, True, pes[18:]))
        receiver.cipher.close(); peer.close()

    def test_device_discovery_clock_and_media_port(self):
        own = dict(deviceId='own-test-device', deviceName='CarLink', deviceType=4)
        session = ControlSession(device_info=own)
        session.negotiated = True
        def business(module, body):
            return pack_frame(b'$' + module.to_bytes(4, 'big') + bytes.fromhex('ffffffff')
                              + len(body).to_bytes(2, 'big') + body)
        session.receive(business(3, pack_uint(101) + json.dumps(dict(deviceId='peer-test', businessId=1)).encode()))
        response = Decoder().feed(session.take_pending()[0])[0].payload[11:]
        command, offset = read_uint(response)
        self.assertEqual(command, 101)
        self.assertEqual(json.loads(response[offset:])['deviceId'], own['deviceId'])
        session.receive(business(3, pack_uint(104) + (831).to_bytes(4, 'big')))
        response = Decoder().feed(session.take_pending()[0])[0].payload[11:]
        command, offset = read_uint(response)
        self.assertEqual(command, 102)
        self.assertEqual(json.loads(response[offset:])['serviceType'], 8)
        session.receive(business(5, pack_uint(101) + pack_uint(49000)))
        self.assertEqual(session.media_port, 49000)
        session.receive(pack_frame(b'SET_PARAMETER * DMSDP/1.0\r\nCSeq: 2\r\n'
            b'TimeServerPort: 49001\r\nTimeUsHigh: 1\r\nTimeUsLow: -1\r\n\r\n'))
        self.assertEqual(session.clock[0], (1 << 32) | 0xffffffff)
        self.assertEqual(len(session.take_pending()), 1)


class HiSightCloseTests(unittest.IsolatedAsyncioTestCase):
    async def test_revoked_rust_worker_does_not_interrupt_cleanup(self):
        class Revoked:
            def teardown(self):
                raise ProtocolError('stale HiSight session')
        receiver, peer = socket.socketpair()
        events = []
        try:
            await graceful_close(receiver, Revoked(), lambda event, **fields: events.append(event))
            self.assertEqual(events, ['hisight_teardown_stopped'])
        finally:
            receiver.close(); peer.close()

    async def test_encrypted_teardown_ack_and_keepalive(self):
        receiver, peer = socket.socketpair()
        receiver.setblocking(False); peer.setblocking(False)
        session = Session(bytes(range(16)))
        session.encrypted = True; session.media_endpoint = 49000
        cipher = Cipher(bytes(range(16)), 65001)
        events = []
        loop = asyncio.get_running_loop()
        task = asyncio.create_task(graceful_close(receiver, session,
            lambda event, **fields: events.append((event, fields)), timeout=1))
        try:
            wire = await asyncio.wait_for(loop.sock_recv(peer, 4096), 1)
            self.assertEqual(int.from_bytes(wire[:4], 'big'), len(wire) - 4)
            request = Messages('RTSP').feed(cipher.decrypt(wire[4:]))[0]
            self.assertTrue(request.start.startswith('TEARDOWN '))
            replies = b''
            for text in (message('GET_PARAMETER * RTSP/1.0', 20),
                         message('RTSP/1.0 200 OK', request.headers['cseq'])):
                data = cipher.encrypt(text)
                replies += len(data).to_bytes(4, 'big') + data
            await loop.sock_sendall(peer, replies)
            await task
            self.assertTrue(session.teardown_acknowledged)
            self.assertIn(('hisight_teardown_done', {'acknowledged': True, 'peer_teardown': False}), events)
            self.assertIsNone(session.teardown())
        finally:
            task.cancel(); await asyncio.gather(task, return_exceptions=True)
            receiver.close(); peer.close(); session.cipher.close(); cipher.close()

    async def test_unresponsive_peer_does_not_block_cleanup(self):
        receiver, peer = socket.socketpair()
        receiver.setblocking(False)
        session = Session(bytes(16))
        session.encrypted = True; session.media_endpoint = 49000
        events = []
        try:
            await asyncio.wait_for(graceful_close(receiver, session,
                lambda event, **fields: events.append(event), timeout=0.02), 1)
            self.assertIn('hisight_teardown_stopped', events)
            self.assertFalse(session.teardown_acknowledged)
        finally:
            receiver.close(); peer.close(); session.cipher.close()

    async def test_source_can_finish_with_its_own_teardown_trigger(self):
        receiver, peer = socket.socketpair()
        receiver.setblocking(False); peer.setblocking(False)
        session = Session(bytes(16)); cipher = Cipher(bytes(16), 65001)
        session.encrypted = True; session.media_endpoint = 49000
        loop = asyncio.get_running_loop()
        task = asyncio.create_task(graceful_close(receiver, session, lambda *a, **kw: None))
        try:
            await asyncio.wait_for(loop.sock_recv(peer, 4096), 1)
            payload = cipher.encrypt(message('SET_PARAMETER * RTSP/1.0', 30,
                                             b'his_trigger_method: TEARDOWN\r\n'))
            await loop.sock_sendall(peer, len(payload).to_bytes(4, 'big') + payload)
            await asyncio.wait_for(task, 1)
            self.assertTrue(session.peer_teardown)
            wire = await asyncio.wait_for(loop.sock_recv(peer, 4096), 1)
            self.assertTrue(cipher.decrypt(wire[4:]).startswith(b'RTSP/1.0 200'))
        finally:
            task.cancel(); await asyncio.gather(task, return_exceptions=True)
            receiver.close(); peer.close(); session.cipher.close(); cipher.close()

    async def test_peer_teardown_is_acknowledged_without_echo(self):
        session = Session(bytes(16)); cipher = Cipher(bytes(16), 65001)
        session.encrypted = True; session.media_endpoint = 49000
        try:
            data = cipher.encrypt(message('SET_PARAMETER * RTSP/1.0', 9,
                                          b'his_trigger_method: TEARDOWN\r\n'))
            session.receive(len(data).to_bytes(4, 'big') + data)
            self.assertTrue(session.peer_teardown)
            self.assertTrue(cipher.decrypt(session.pending[0][4:]).startswith(b'RTSP/1.0 200'))
            self.assertIsNone(session.teardown())
        finally:
            session.cipher.close(); cipher.close()
