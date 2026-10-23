import json
import unittest
from auth_wire import AuthError, AuthSession, JsonStream, MAX_MESSAGE
from identity import Identity


def hello(**changes):
    data = dict(deviceId='a' * 64, devicename='test phone', has_pin=True, usb_connection=False)
    data.update(changes)
    return dict(version='14.2.0.400', oper_type=1, data=json.dumps(data))


class AuthWireTests(unittest.TestCase):
    def test_utf8_splits_escapes_and_combined_objects(self):
        obj = dict(text='华为 { " \\ }', nested=[dict(a=1)])
        wire = (json.dumps(obj, ensure_ascii=False) + '\n' + json.dumps(hello())).encode('utf-8')
        for cut in range(len(wire) + 1):
            stream = JsonStream()
            self.assertEqual(stream.feed(wire[:cut]) + stream.feed(wire[cut:]), [obj, hello()])
            stream.finish()

    def test_invalid_or_incomplete_stream(self):
        for wire in (b'[]', b'{"x":1,"x":2}', b'{"x":NaN}', b'{"x":\xff}',
                     b'{"x":"' + b'x' * MAX_MESSAGE + b'"}'):
            with self.subTest(wire=wire[:20]), self.assertRaises(AuthError):
                JsonStream().feed(wire)
        stream = JsonStream()
        stream.feed(b'{"x":')
        with self.assertRaises(AuthError):
            stream.finish()

    def test_handshake_does_not_transmit_pin(self):
        identity = Identity('b' * 64, '000042')
        session = AuthSession(identity, '00112233')
        wire, meta = session.receive(hello())
        self.assertEqual(meta['operation'], 1)
        self.assertNotIn(identity.pin.encode(), wire)
        data = json.loads(json.loads(wire)['data'])
        self.assertEqual(data['deviceId'], identity.device_id)
        self.assertTrue(data['has_pin'])
        with self.assertRaises(AuthError):
            session.receive(hello())

    def test_auth_requires_handshake_and_never_fakes_success(self):
        session = AuthSession(Identity('b' * 64, '000042'), '00112233')
        auth = dict(oper_type=3, data=json.dumps(dict(message=1, payload={})))
        with self.assertRaises(AuthError):
            session.receive(auth)
        session.receive(hello())
        reply, meta = session.receive(auth)
        self.assertIsNone(reply)
        self.assertEqual(meta, dict(operation=3, message=1, backend='not_connected'))
        for invalid in (hello(deviceId='a' * 65), hello(usb_connection=True)):
            with self.assertRaises(AuthError):
                AuthSession(session.identity, '00112233').receive(invalid)

    def test_native_packets_are_wrapped_and_session_closed(self):
        class Backend:
            results = [2]
            session_key = None
            closed = False

            def receive(self, data):
                self.request = json.loads(data)
                return 0, [b'{"message":32769,"payload":{}}', b'{"message":32770}']

            def close(self):
                self.closed = True
        backend = Backend()
        calls = []
        def factory(*params):
            calls.append(params)
            return backend
        identity = Identity('b' * 64, '000042')
        session = AuthSession(identity, '00112233', factory)
        session.receive(hello())
        self.assertEqual(calls, [(identity.device_id, 'a' * 64, identity.pin)])
        reply, meta = session.receive(dict(oper_type=3, data='{"message":1}'))
        frames = JsonStream().feed(reply)
        self.assertEqual([json.loads(f['data'])['message'] for f in frames], [32769, 32770])
        self.assertNotIn(identity.pin.encode(), reply)
        self.assertEqual(meta['replies'], 2)
        self.assertFalse(meta['key_ready'])
        session.close()
        self.assertTrue(backend.closed)
