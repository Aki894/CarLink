import unittest
from control_wire import Decoder, Messages, Session, encode, pack_frame
from cipher import Cipher
from nearby_wire import ProtocolError


class ControlTests(unittest.TestCase):
    def test_no_heartbeat_or_speaker_before_negotiation(self):
        session = Session(audio=True)
        with self.assertRaises(ProtocolError):
            session.heartbeat()
        with self.assertRaises(ProtocolError):
            session.receive(pack_frame(encode('GET_PARAMETER * DMSDP/1.0',
                dict(ServiceID='speaker1', ServiceType=3, DataSessionID=42, CSeq=1))))
        self.assertIsNone(session.speaker.session_id)

    def test_initial_offer_has_only_implemented_capabilities(self):
        wire = Session().initial()
        frame = Decoder().feed(wire)[0]
        message = Messages().feed(frame.payload)[0]
        self.assertEqual(message.start, 'OPTIONS * DMSDP/1.0')
        self.assertEqual(message.headers['cryptoability'], '222')
        self.assertEqual(message.headers['cryptoalg'], 'null')
        self.assertEqual(message.headers['udpka'], '0')

    def test_frame_splits_and_coalescing(self):
        wire = pack_frame(b'example') + pack_frame(b'encrypted example', 0xab)
        for split in range(len(wire) + 1):
            decoder = Decoder()
            frames = decoder.feed(wire[:split]) + decoder.feed(wire[split:])
            self.assertEqual([f.marker for f in frames], [0, 0xab])
            self.assertEqual([f.payload for f in frames], [b'example', b'encrypted example'])
            decoder.finish()

    def test_message_split_body_and_secret_metadata(self):
        wire = (b'DMSDP/1.0 200 OK\r\nCSeq: 1\r\nPeerKey: private-example\r\n'
                b'Content-Length: 4\r\n\r\nbody')
        for split in range(len(wire) + 1):
            decoder = Messages()
            messages = decoder.feed(wire[:split]) + decoder.feed(wire[split:])
            self.assertEqual(len(messages), 1)
            self.assertEqual(messages[0].body, b'body')
            metadata = messages[0].metadata()
            self.assertNotIn('private-example', repr(metadata))
            self.assertNotIn('private-example', repr(messages[0]))
            self.assertEqual(metadata['cseq'], 1)

    def test_invalid_framing_and_headers_fail_closed(self):
        for wire in [b'\x01\x00\x01a', b'\x00\x00\x00', b'\x00\xff\xff']:
            decoder = Decoder()
            with self.assertRaises(ProtocolError):
                decoder.feed(wire)
            with self.assertRaises(ProtocolError):
                decoder.feed(pack_frame(b'valid'))
        for headers in [b'CSeq:1\r\ncseq:2', b'Content-Length:-1', b'Content-Length:99999',
                        b'bad header:1']:
            with self.assertRaises(ProtocolError):
                Messages().feed(b'OPTIONS * DMSDP/1.0\r\n' + headers + b'\r\n\r\n')
        with self.assertRaises(ProtocolError):
            encode('OPTIONS * DMSDP/1.0', {'CSeq': '1\r\nInjected: true'})

    def test_encrypted_frame_is_not_parsed_as_text(self):
        self.assertEqual(Session().receive(pack_frame(b'private ciphertext', 0xab)),
                         [dict(encrypted=True, size=18)])

    def test_negotiation_acknowledges_peer_sequence(self):
        request = (b'SET_PARAMETER * DMSDP/1.0\r\nCSeq: 0\r\nCryptoVer: 1\r\n'
                   b'CryptoSwitch: 222\r\nCryptoAlg: null\r\n\r\n')
        wire = pack_frame(request)
        for split in range(len(wire) + 1):
            session = Session()
            events = session.receive(wire[:split]) + session.receive(wire[split:])
            self.assertIn(dict(negotiated=True, cipher='null'), events)
            replies = session.take_pending()
            self.assertEqual(len(replies), 1)
            response = Messages().feed(Decoder().feed(replies[0])[0].payload)[0]
            self.assertEqual(response.start, 'DMSDP/1.0 200 OK')
            self.assertEqual(response.headers['cseq'], '0')
            self.assertEqual(session.take_pending(), [])
            with self.assertRaises(ProtocolError):
                session.receive(wire)

    def test_unsupported_cipher_is_not_acknowledged(self):
        for replacement in (b'CryptoAlg: aes128-ctr', b'CryptoSwitch: 0'):
            request = (b'SET_PARAMETER * DMSDP/1.0\r\nCSeq: 0\r\nCryptoVer: 1\r\n'
                       b'CryptoSwitch: 222\r\nCryptoAlg: null\r\n\r\n')
            old = b'CryptoAlg: null' if replacement.startswith(b'CryptoAlg') else b'CryptoSwitch: 222'
            session = Session()
            with self.assertRaises(ProtocolError):
                session.receive(pack_frame(request.replace(old, replacement)))
            self.assertFalse(session.negotiated)
            self.assertEqual(session.take_pending(), [])

    def test_binary_control_is_not_text_or_body_logging(self):
        body = b'private example'
        payload = b'$' + (3).to_bytes(4, 'big') + (110).to_bytes(4, 'big')
        payload += len(body).to_bytes(2, 'big') + body
        self.assertEqual(Session().receive(pack_frame(payload)),
                         [dict(binary=True, module=3, data_type=110, body_bytes=len(body))])
        with self.assertRaises(ProtocolError):
            Session().receive(pack_frame(payload[:-1]))

    def test_encrypted_negotiation_keepalive_and_later_parameters(self):
        key = bytes(range(32))
        session, peer = Session(key), Cipher(key)
        session.receive(pack_frame(b'SET_PARAMETER * DMSDP/1.0\r\nCSeq: 0\r\n'
                                   b'CryptoVer: 1\r\nCryptoSwitch: 222\r\n'
                                   b'CryptoAlg: aes128-ctr\r\n\r\n'))
        self.assertEqual(Decoder().feed(session.take_pending()[0])[0].marker, 0)
        request = b'GET_PARAMETER * DMSDP/1.0\r\nCSeq: 1\r\n\r\n'
        events = session.receive(pack_frame(peer.encrypt(request), 0xab))
        self.assertEqual(events[0]['method'], 'GET_PARAMETER')
        self.assertIn(dict(keepalive_received=True), events)
        self.assertEqual(session.take_pending(), [])
        response = Decoder().feed(session.heartbeat())[0]
        self.assertEqual(response.marker, 0xab)
        heartbeat = Messages().feed(peer.decrypt(response.payload))[0]
        self.assertEqual(heartbeat.start, 'GET_PARAMETER * DMSDP/1.0')
        self.assertEqual(heartbeat.headers['cseq'], '2')
        # Ordinary service parameters are observed, never misclassified as
        # repeated crypto negotiation or acknowledged before implementation.
        events = session.receive(pack_frame(peer.encrypt(
            b'SET_PARAMETER * DMSDP/1.0\r\nCSeq: 2\r\nService: example\r\n\r\n'), 0xab))
        self.assertEqual(events[0]['method'], 'SET_PARAMETER')
        self.assertEqual(session.take_pending(), [])
        session.close()
        peer.close()
