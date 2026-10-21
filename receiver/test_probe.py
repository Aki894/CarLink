"""Protocol checks; run with python3 -m unittest discover -s receiver."""
import socket
import struct
import unittest

import probe


class WireTests(unittest.TestCase):
    def test_upstream_video_vector(self):
        from pathlib import Path
        path = Path(__file__).resolve().parents[1] / 'research/repos/OpenXPlay-Stream/spec/vectors/wire/video.hex'
        if not path.exists():
            self.skipTest('optional pinned upstream vectors absent')
        data = bytes.fromhex(path.read_text())
        # Upstream vectors include the little-endian stream length prefix.
        left, right = socket.socketpair()
        with left, right:
            left.sendall(data)
            message = probe.receive(right)
            self.assertEqual(message[0], 1)

    def test_json_roundtrip(self):
        left, right = socket.socketpair()
        with left, right:
            value = dict(type='hello', version=0, role='producer', name='test', client='test')
            probe.send_json(left, value)
            self.assertEqual(probe.receive(right), value)

    def test_reject_size_before_allocation(self):
        left, right = socket.socketpair()
        with left, right:
            left.sendall(struct.pack('<I', probe.MAX_FRAME + 1))
            with self.assertRaises(ValueError):
                probe.receive(right)

    def test_truncated_frame(self):
        left, right = socket.socketpair()
        with left, right:
            left.sendall(struct.pack('<I', 10) + b'a')
            left.shutdown(socket.SHUT_WR)
            with self.assertRaises(EOFError):
                probe.receive(right)

    def test_annexb(self):
        self.assertEqual(probe.annexb(b'\0\0\0\2\x65\x80'), b'\0\0\0\1\x65\x80')
        for malformed in (b'', b'\0', b'\0\0\0\0', b'\0\0\0\3\x65'):
            with self.assertRaises(ValueError):
                probe.annexb(malformed)

    def test_avc_sps_pps(self):
        config = bytes.fromhex('0142c01effe1000267800100026880')
        self.assertEqual(probe.avc_parameters(config), b'\0\0\0\1\x67\x80\0\0\0\1\x68\x80')
        for malformed in (b'', config[:-1], config[:5] + b'\xe0' + config[6:]):
            with self.assertRaises(ValueError):
                probe.avc_parameters(malformed)

    def test_keyframe_and_touch(self):
        left, right = socket.socketpair()
        with left, right:
            probe.control(left, dict(type='keyframe', screen=0))
            self.assertEqual(probe.receive(right), dict(type='keyframe', screen=0))
            probe.control(left, dict(type='touch', contacts=[dict(id=0, x=0.5, y=1, down=True)]))
            message = probe.receive(right)
            self.assertEqual(message[0], 16)
            self.assertEqual(message[-1], b'\x01\x00\x01\x00\x80\xff\xff')

    def test_invalid_controls(self):
        left, right = socket.socketpair()
        with left, right:
            for c in (dict(type='restart'), dict(type='touch', contacts=[dict(id=0, x=float('nan'), y=0, down=True)]),
                      dict(type='touch', contacts=[dict(id=0, x=0, y=0, down=True)] * 2)):
                with self.assertRaises(ValueError):
                    probe.control(left, c)


if __name__ == '__main__':
    unittest.main()
