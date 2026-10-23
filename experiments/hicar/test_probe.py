import asyncio
import contextlib
import io
import socket
import unittest
import xml.etree.ElementTree as ET

from probe import Advertisement, FE35, PeerAgent, Profile, beacon, observe, service_record
from dbus_next import DBusError


class BeaconTests(unittest.TestCase):
    def test_layout_and_leading_zeroes(self):
        self.assertEqual(beacon('00112233'), bytes.fromhex('0101010300112204331001'))
        self.assertEqual(beacon('00112233', 2)[-1], 2)
        for sequence in (-1, 256):
            with self.assertRaises(ValueError):
                beacon('00112233', sequence)

    def test_invalid_models(self):
        for model in ('', '001122', '0011223344', '0011zz33', '00 11 22'):
            with self.subTest(model=model), self.assertRaises(ValueError):
                beacon(model)

    def test_bluez_interfaces_have_expected_signatures(self):
        stop = asyncio.Event()
        profile = Profile(stop, None).introspect()
        method = next(m for m in profile.methods if m.name == 'NewConnection')
        self.assertEqual(method.in_signature, 'oha{sv}')
        advertisement = Advertisement(beacon('00112233'), stop).introspect()
        self.assertEqual({p.name for p in advertisement.properties},
                         {'Type', 'Discoverable', 'TxPower', 'Instance', 'ServiceData'})

    def test_sdp_matches_channel_and_service(self):
        record = ET.fromstring(service_record(23))
        self.assertEqual(record.find("attribute[@id='0x0001']/sequence/uuid").get('value'), FE35)
        self.assertEqual(record.find("attribute[@id='0x0004']/sequence/sequence/uint8").get('value'), '23')
        for channel in (0, 31):
            with self.assertRaises(ValueError):
                service_record(channel)

    def test_pairing_agent_only_accepts_selected_phone(self):
        selected = '/org/bluez/hci0/dev_00_11_22_33_44_55'
        agent = PeerAgent(selected)
        with contextlib.redirect_stdout(io.StringIO()):
            agent.RequestConfirmation(selected, 123456)
        agent.AuthorizeService(selected, FE35)
        with self.assertRaises(DBusError):
            agent.RequestConfirmation('/org/bluez/hci0/dev_00_11_22_33_44_66', 123456)
        with self.assertRaises(DBusError):
            agent.AuthorizeService(selected, 'unsupported-service')


class StreamTests(unittest.IsolatedAsyncioTestCase):
    async def test_fragmented_stream_and_eof(self):
        receiver, sender = socket.socketpair()
        with contextlib.redirect_stdout(io.StringIO()) as output:
            task = asyncio.create_task(observe(receiver, 1))
            try:
                sender.sendall(b'hello')
                await asyncio.sleep(0.01)
                sender.sendall(b'world')
                sender.close()
                await asyncio.wait_for(task, 1)
            finally:
                sender.close()
        self.assertIn('"bytes": 10', output.getvalue())
        self.assertEqual(receiver.fileno(), -1)

    async def test_stream_limit_closes_without_eof(self):
        receiver, sender = socket.socketpair()
        try:
            with contextlib.redirect_stdout(io.StringIO()) as output:
                task = asyncio.create_task(observe(receiver, 2, limit=8))
                sender.sendall(b'0123456789')
                await asyncio.wait_for(task, 1)
            self.assertIn('"event": "capture_limit"', output.getvalue())
            self.assertIn('"bytes": 8', output.getvalue())
            self.assertEqual(receiver.fileno(), -1)
        finally:
            sender.close()

    async def test_cancel_releases_socket(self):
        receiver, sender = socket.socketpair()
        try:
            with contextlib.redirect_stdout(io.StringIO()):
                task = asyncio.create_task(observe(receiver, 3))
                await asyncio.sleep(0)
                task.cancel()
                with self.assertRaises(asyncio.CancelledError):
                    await task
            self.assertEqual(receiver.fileno(), -1)
        finally:
            sender.close()
