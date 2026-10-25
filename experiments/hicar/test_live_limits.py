"""Managed transports outlive capture budgets while capture files stay bounded."""
import asyncio
from pathlib import Path
import socket
import tempfile
import unittest
from unittest import mock

import hisight
import probe


class FakeCipher:
    def close(self):
        pass


class PassiveSession:
    """Transport-only fixture: protocol/cipher behavior has separate tests."""
    speaker = clock = media_port = media_endpoint = input_endpoint = None
    negotiated = peer_teardown = teardown_acknowledged = False

    def __init__(self, *_):
        self.pending = []
        self.cipher = FakeCipher()
        self.on_plaintext = None

    def initial(self):
        return b''

    def receive(self, data):
        if self.on_plaintext:
            self.on_plaintext(data)
        return [], []

    def take_pending(self):
        return []

    def teardown(self):
        return None

    def close(self):
        pass


class ProbeSession(PassiveSession):
    def receive(self, data):
        super().receive(data)
        return []


class LiveLimitsTests(unittest.IsolatedAsyncioTestCase):
    async def exchange(self, receive, continuous, capture):
        reader, writer = socket.socketpair()
        reader.setblocking(False)
        writer.setblocking(False)
        sent = 1024 * 1024 + 128 * 1024
        payload = b'transport-fixture-' * (sent // 18 + 1)
        payload = payload[:sent]
        self.assertEqual(len(payload), sent)
        logs = []

        def log(event, **fields):
            if event in ('stream_closed', 'hisight_closed', 'capture_limit'):
                logs.append((event, fields))

        async def produce():
            try:
                await asyncio.get_running_loop().sock_sendall(writer, payload)
                writer.shutdown(socket.SHUT_WR)
            except (BrokenPipeError, ConnectionResetError, OSError):
                if continuous:
                    raise

        with tempfile.TemporaryDirectory(prefix='carlink-live-limits-') as directory:
            path = directory if capture else None
            if receive == 'probe':
                operation = probe.observe(reader, 'fixture', path, control=ProbeSession(),
                                          decode_control=False, continuous=continuous)
                patch = mock.patch.object(probe, 'log', side_effect=log)
            else:
                operation = hisight.receive(reader, bytes(range(16)), path, log,
                                            continuous=continuous)
                patch = mock.patch.object(hisight, 'Session', PassiveSession)
            try:
                with patch:
                    consumer = asyncio.create_task(operation)
                    producer = asyncio.create_task(produce())
                    await asyncio.wait_for(consumer, 10)
                    reader.close()  # The HiSight caller owns its socket.
                    await asyncio.wait_for(producer, 5)
                final = next(fields for event, fields in logs
                             if event == ('stream_closed' if receive == 'probe' else 'hisight_closed'))
                self.assertEqual(final['bytes'], sent if continuous else 1024 * 1024)
                self.assertEqual(any(event == 'capture_limit' for event, _ in logs),
                                 receive == 'probe' and not continuous)
                files = list(Path(directory).iterdir())
                if capture:
                    self.assertEqual(len(files), 2)
                    self.assertTrue(all(0 < p.stat().st_size <= 1024 * 1024 for p in files))
                    raw = next(p for p in files if p.name.endswith('.bin'))
                    self.assertEqual(raw.stat().st_size, 1024 * 1024)
                else:
                    self.assertEqual(files, [])
            finally:
                reader.close()
                writer.close()
                for task in (locals().get('consumer'), locals().get('producer')):
                    if task and not task.done():
                        task.cancel()
                        await asyncio.gather(task, return_exceptions=True)

    async def test_managed_probe_continues_after_one_mib_and_bounds_both_captures(self):
        await self.exchange('probe', True, True)

    async def test_experimental_probe_still_stops_at_original_byte_budget(self):
        await self.exchange('probe', False, False)

    async def test_managed_hisight_continues_after_one_mib_and_bounds_both_captures(self):
        await self.exchange('hisight', True, True)

    async def test_experimental_hisight_still_stops_at_original_byte_budget(self):
        await self.exchange('hisight', False, False)


if __name__ == '__main__':
    unittest.main()
