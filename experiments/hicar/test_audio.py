import asyncio
import socket
import struct
import time
import tempfile
from pathlib import Path
import unittest
from audio_wire import Speaker
from audio_transport import unpack_datagram, synchronize, Clock, Receiver
from audio_media import Audio, adts_frames, rtp_audio
from cipher import Cipher
from control_wire import Message, Messages, encode, pack_frame
from nearby_wire import ProtocolError


def command(method='SET_PARAMETER', sequence=1, body=b'', **headers):
    return Message(method + ' * DMSDP/1.0', dict(serviceid='speaker1', servicetype='3',
        datasessionid='42', cseq=str(sequence), **headers), body)


class SpeakerTests(unittest.TestCase):
    def test_simple_trigger_negotiates_in_setup(self):
        speaker = Speaker()
        speaker.ports = (40000, -1)
        self.assertEqual(speaker.receive(command(body=b'msdp_trigger_method: OPTIONS\r\n', simple='1'), encode),
                         [dict(audio_setup_sent=True)])
        request = Messages().feed(speaker.pending.pop())[0]
        self.assertEqual(request.start, 'SETUP * DMSDP/1.0')
        self.assertEqual(request.headers['simple'], '1')
        self.assertEqual(request.headers['codec'], '0')
        self.assertIn('client_port=40000--1', request.headers['transport'])
        response = Message('DMSDP/1.0 200 OK', dict(serviceid='speaker1', servicetype='3',
                           datasessionid='42', cseq=str(speaker.setup_sequence),
                           transport='RTP/AVP/UDP;unicast;client_port=40000--1;server_port=44000--1'), b'')
        self.assertEqual(speaker.receive(response, encode), [dict(audio_ready=True)])
        self.assertEqual(speaker.pending, [])
        event = speaker.receive(command(audioparameter='SampleRate=48000;Format=16;ChannelNum=2;Codec=0'), encode)
        self.assertEqual(event[0]['codec'], 'aac-adts')
        self.assertTrue(speaker.ready)
        speaker.pending.clear()
        self.assertEqual(speaker.receive(command(body=b'msdp_trigger_method: TEARDOWN'), encode),
                         [dict(audio_stopped=True)])
        self.assertTrue(speaker.pending[0].startswith(b'TEARDOWN '))
        self.assertIsNone(speaker.teardown(encode))

    def test_invalid_endpoint_mode_parameters_and_session(self):
        speaker = Speaker()
        speaker.ports = (40000, 40001)
        with self.assertRaises(ProtocolError):
            speaker.receive(command(body=b'msdp_trigger_method: OPTIONS', simple='1'), encode)
        speaker.ports = (40000, -1)
        speaker.receive(command(body=b'msdp_trigger_method: OPTIONS', simple='1'), encode)
        for value in ('SampleRate=48000;Format=16;ChannelNum=2;Codec=2',
                      'SampleRate=48000;Format=16;ChannelNum=99;Codec=0', ''):
            with self.assertRaises(ProtocolError):
                speaker.receive(command(audioparameter=value), encode)
        bad = command(audioparameter='SampleRate=48000;Format=16;ChannelNum=2;Codec=0')
        bad.headers['datasessionid'] = '43'
        with self.assertRaises(ProtocolError):
            speaker.receive(bad, encode)
        self.assertFalse(speaker.ready)

    def test_unrelated_and_invalid_control(self):
        speaker = Speaker()
        self.assertIsNone(speaker.receive(Message('GET_PARAMETER * DMSDP/1.0', {'cseq':'1'}, b''), encode))
        for headers in ({'cseq':'bad'}, {'datasessionid':'-1'}, {'servicetype':'9'}):
            message = command()
            message.headers.update(headers)
            with self.assertRaises(ProtocolError):
                speaker.receive(message, encode)
        with self.assertRaises(ProtocolError):
            speaker.receive(command(body=b'msdp_trigger_method: OPTIONS'), encode)

    def test_private_udp_envelope(self):
        cipher = Cipher(bytes(range(16)))
        try:
            packet = pack_frame(cipher.encrypt(b'timestamp'), 0xab)
            self.assertEqual(unpack_datagram(packet, cipher), b'timestamp')
            for bad in (packet[:-1], b'\0'+packet[1:], b''):
                with self.assertRaises(ProtocolError):
                    unpack_datagram(bad, cipher)
        finally:
            cipher.close()


class ClockTests(unittest.IsolatedAsyncioTestCase):
    def test_wrap_and_stale_clock(self):
        clock = Clock()
        self.assertIsNone(clock.local_rtp_time(1, 100))
        now, offset = 900000000000, 123456789
        clock.update(offset, 1000, now)
        timestamp = ((now+offset)*90000//1000000) & 0xffffffff
        self.assertLess(abs(clock.local_rtp_time(timestamp, now)-now), 12)
        self.assertIsNone(clock.local_rtp_time(timestamp, now+60000001))

    async def test_encrypted_udp_clock_and_cancel(self):
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.setblocking(False)
        sock.bind(('127.0.0.1', 0))
        cipher = Cipher(bytes(range(16)))
        records, done = [], asyncio.Event()

        async def server():
            loop = asyncio.get_running_loop()
            for _ in range(5):
                data, address = await loop.sock_recvfrom(sock, 1500)
                t1, _, _, _ = struct.unpack('<qqqq', unpack_datagram(data, cipher))
                t2 = time.monotonic_ns()//1000 + 500000
                response = struct.pack('<qqqq', t1, t2, t2, 0)
                await loop.sock_sendto(sock, pack_frame(cipher.encrypt(response), 0xab), address)

        def log(event, **fields):
            records.append((event, fields))
            done.set()

        responder = asyncio.create_task(server())
        task = asyncio.create_task(synchronize('127.0.0.1', '127.0.0.1',
                                               sock.getsockname()[1], bytes(range(16)), log))
        try:
            await asyncio.wait_for(done.wait(), 3)
            await responder
            self.assertEqual(records[0][0], 'clock_synchronized')
            self.assertEqual(records[0][1]['samples'], 5)
            self.assertLess(abs(records[0][1]['offset_us']-500000), 50000)
        finally:
            task.cancel()
            responder.cancel()
            await asyncio.gather(task, responder, return_exceptions=True)
            sock.close()
            cipher.close()

    async def test_receiver_extracts_private_aac_and_survives_bad_packet(self):
        with tempfile.TemporaryDirectory() as directory:
            events, done = [], asyncio.Event()
            def log(event, **fields):
                events.append(event)
                if event == 'audio_datagram' and fields['packets'] == 3:
                    done.set()
            receiver = Receiver('127.0.0.1','127.0.0.1',bytes(range(16)),directory,log)
            sender = socket.socket(socket.AF_INET,socket.SOCK_DGRAM)
            sender.setblocking(False)
            cipher = Cipher(bytes(range(16)))
            receiver.begin()
            receiver.active = True
            task = asyncio.create_task(receiver.receive(0))
            try:
                self.assertEqual(receiver.ports[1], -1)
                loop = asyncio.get_running_loop()
                for data in (packet(0), b'', packet(1)):
                    wire = pack_frame(cipher.encrypt(data),0xab) if data else b''
                    await loop.sock_sendto(sender,wire,('127.0.0.1',receiver.ports[0]))
                await asyncio.wait_for(done.wait(), 3)
                self.assertEqual(receiver.audio.frames,2)
                self.assertIn('audio_packet_rejected',events)
            finally:
                task.cancel()
                await asyncio.gather(task,return_exceptions=True)
                receiver.close()
                sender.close()
                cipher.close()
            self.assertEqual(list(Path(directory).glob('*.aac'))[0].read_bytes(),aac_frame()*2)


def aac_frame(payload=b'\x00\x00\x00\x00'):
    length = 7+len(payload)
    return bytes((255,241,76,128 | (length>>11), (length>>3)&255,
                  ((length&7)<<5)|31,252))+payload


def packet(sequence, data=None, timestamp=100, stream=1):
    return struct.pack('!BBHII',128,96,sequence,timestamp,stream)+(data or aac_frame())


class AudioMediaTests(unittest.TestCase):
    def test_adts_batch_and_rtp_timestamp_is_not_dedup_key(self):
        raw = aac_frame()
        self.assertEqual(adts_frames(raw+raw), [(raw,48000,2,1024)]*2)
        audio = Audio()
        self.assertEqual(audio.receive(packet(0))[0]['timestamp_offset_samples'], 0)
        self.assertEqual(audio.receive(packet(1))[0]['timestamp_offset_samples'], 1024)
        self.assertEqual(audio.frames, 2)
        self.assertEqual(audio.receive(packet(1)), [])
        self.assertEqual(audio.duplicates, 1)

    def test_reorder_gap_wrap_and_bounded_streams(self):
        audio = Audio()
        audio.receive(packet(65535))
        self.assertEqual(audio.receive(packet(1)), [])
        self.assertEqual(len(audio.receive(packet(0))), 2)
        self.assertEqual(audio.receive(packet(40))[0]['rtp_timestamp'], 100)
        self.assertEqual(audio.lost, 38)
        for stream in range(2,9):
            audio.receive(packet(0, stream=stream))
        with self.assertRaises(ProtocolError):
            audio.receive(packet(0, stream=9))

    def test_truncation_invalid_adts_and_rtp_extensions(self):
        good = packet(0)
        for size in range(len(good)):
            with self.assertRaises(ProtocolError):
                rtp_audio(good[:size])
        for data in (bytes(12), good[:1]+b'\x61'+good[2:],
                     b'\x90'+good[1:12]+b'\0\0\xff\xff'+good[12:],
                     b'\xa0'+good[1:]+b'\0'):
            with self.assertRaises(ProtocolError):
                rtp_audio(data)
