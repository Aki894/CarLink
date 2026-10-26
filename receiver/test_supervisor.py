import errno
import json
import os
from pathlib import Path
import socket
import tempfile
import unittest
from unittest.mock import patch

import supervisor


class BootstrapTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='carlink-bootstrap-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.example = self.root / 'example.json'
        self.example.write_text(json.dumps({'schema': 1, 'devices': []}))
        self.target = self.root / 'etc/carlink/manager.json'

    def test_publish_once_preserves_existing_configuration(self):
        self.assertTrue(supervisor.bootstrap_config(self.target, self.example))
        self.assertEqual(self.target.read_bytes(), self.example.read_bytes())
        self.target.write_text('local configuration')
        self.assertFalse(supervisor.bootstrap_config(self.target, self.example))
        self.assertEqual(self.target.read_text(), 'local configuration')
        self.assertEqual(list(self.target.parent.glob('.manager-*')), [])
        if os.name == 'posix':
            self.assertEqual(self.target.stat().st_mode & 0o777, 0o600)

    def test_reject_invalid_example_without_publishing(self):
        self.example.write_text('{"schema": 2}')
        with self.assertRaises(ValueError):
            supervisor.bootstrap_config(self.target, self.example)
        self.assertFalse(self.target.exists())


@unittest.skipUnless(os.name == 'posix', 'Unix socket ownership requires POSIX')
class SocketTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='carlink-socket-')
        self.addCleanup(self.temp.cleanup)
        self.path = str(Path(self.temp.name) / 'source.sock')

    def bind(self, listen=False):
        server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.addCleanup(server.close)
        server.bind(self.path)
        if listen:
            server.listen(2)
        return server

    def test_dead_socket_removed(self):
        self.bind().close()
        self.assertTrue(supervisor.clear_stale_socket(self.path))
        self.assertFalse(os.path.lexists(self.path))
        self.assertFalse(supervisor.clear_stale_socket(self.path))

    def test_listening_socket_preserved(self):
        self.bind(listen=True)
        with self.assertRaises(RuntimeError):
            supervisor.clear_stale_socket(self.path)
        self.assertTrue(os.path.lexists(self.path))

    def test_regular_file_and_symlink_preserved(self):
        Path(self.path).write_text('keep')
        with self.assertRaises(RuntimeError):
            supervisor.clear_stale_socket(self.path)
        self.assertEqual(Path(self.path).read_text(), 'keep')
        os.unlink(self.path)
        os.symlink('missing-target', self.path)
        with self.assertRaises(RuntimeError):
            supervisor.clear_stale_socket(self.path)
        self.assertTrue(os.path.islink(self.path))

    def test_owner_change_preserved(self):
        self.bind().close()
        with patch.object(supervisor.os, 'geteuid', return_value=os.geteuid()+1):
            with self.assertRaises(RuntimeError):
                supervisor.clear_stale_socket(self.path)
        self.assertTrue(os.path.lexists(self.path))

    def test_replaced_inode_preserved(self):
        self.bind().close()
        real_stat = os.lstat
        calls = 0

        def swapped(name):
            nonlocal calls
            calls += 1
            if calls == 2:
                # Replacing with a file is enough to exercise the TOCTOU guard.
                os.unlink(name)
                Path(name).write_text('replacement')
            return real_stat(name)

        with patch.object(supervisor.os, 'lstat', side_effect=swapped):
            with self.assertRaises(RuntimeError):
                supervisor.clear_stale_socket(self.path)
        self.assertEqual(Path(self.path).read_text(), 'replacement')


if __name__ == '__main__':
    unittest.main()
