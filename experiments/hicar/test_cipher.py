import unittest
from cipher import Cipher
from nearby_wire import ProtocolError


class CipherTests(unittest.TestCase):
    def test_nist_sp800_38a_ctr_vector(self):
        cipher = Cipher(bytes.fromhex('2b7e151628aed2a6abf7158809cf4f3c'))
        iv = bytes.fromhex('f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff')
        plain = bytes.fromhex('6bc1bee22e409f96e93d7e117393172a'
                              'ae2d8a571e03ac9c9eb76fac45af8e51')
        expected = bytes.fromhex('874d6191b620e3261bef6864990db6ce'
                                 '9806f66b7970fdff8617187bb9fffdff')
        self.assertEqual(cipher.transform(plain, iv), expected)
        self.assertEqual(cipher.decrypt(iv + expected), plain)
        cipher.close()

    def test_record_random_iv_key_prefix_and_lifetime(self):
        key = bytes(range(32))
        cipher, peer = Cipher(key), Cipher(key[:16])
        first, second = cipher.encrypt(b'example'), cipher.encrypt(b'example')
        self.assertNotEqual(first[:16], second[:16])
        self.assertEqual(peer.decrypt(first), b'example')
        self.assertEqual(peer.decrypt(second), b'example')
        for record in (b'', bytes(16), bytes(1500)):
            with self.assertRaises(ProtocolError):
                cipher.decrypt(record)
        cipher.close()
        with self.assertRaises(ProtocolError):
            cipher.encrypt(b'closed')
        peer.close()
