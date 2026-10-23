import os
from pathlib import Path
import tempfile
import unittest
from identity import Identity, load, load_or_create, set_pin


class IdentityTests(unittest.TestCase):
    def test_pin_and_identity_persist_across_reload(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'private' / 'identity.json'
            first = load_or_create(path)
            self.assertEqual(len(first.pin), 6)
            self.assertEqual(first, load_or_create(path))
            self.assertEqual(first, load(path))
            self.assertNotIn(first.pin, repr(first))
            if os.name == 'posix':
                self.assertEqual(path.stat().st_mode & 0o777, 0o600)

    def test_change_keeps_identity_and_leading_zeroes(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'identity.json'
            first = load_or_create(path)
            changed = set_pin(path, '000042')
            self.assertEqual(changed.pin, '000042')
            self.assertEqual(changed.device_id, first.device_id)
            self.assertEqual(changed, load(path))
            before = path.read_bytes()
            for pin in ('12345', '1234567', '１２３４５６', 'abc123', 123456):
                with self.subTest(pin=pin), self.assertRaises(ValueError):
                    set_pin(path, pin)
                self.assertEqual(path.read_bytes(), before)

    def test_corrupt_file_does_not_regenerate_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'identity.json'
            load_or_create(path)
            path.write_bytes(b'{}')
            with self.assertRaises(ValueError):
                load_or_create(path)
            self.assertEqual(path.read_bytes(), b'{}')
