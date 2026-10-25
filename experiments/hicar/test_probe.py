import asyncio
import contextlib
import io
import os
import socket
import sys
import unittest
from unittest import mock
import xml.etree.ElementTree as ET

from probe import Advertisement, FE35, PeerAgent, PeerSelection, Profile, beacon, observe, service_record, close_tcp_streams, normalize_peer, main
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
                         {'Type', 'Discoverable', 'LocalName', 'TxPower', 'Instance', 'ServiceData'})

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


class PairWindowTests(unittest.TestCase):
    adapter = '/org/bluez/hci0'
    first = adapter + '/dev_00_11_22_33_44_55'
    second = adapter + '/dev_00_11_22_33_44_66'

    def test_agent_candidates_do_not_lock_out_the_transport_peer(self):
        now = [0.0]
        selection = PeerSelection(self.adapter, pair_seconds=5, clock=lambda: now[0])
        agent = PeerAgent(selection)
        with contextlib.redirect_stdout(io.StringIO()) as output:
            agent.RequestConfirmation(self.first, 123456)
            agent.AuthorizeService(self.second, FE35)
            self.assertIsNone(selection.selected)
            self.assertTrue(selection.allows(self.second, select=True))
        self.assertEqual(selection.selected, self.second)
        self.assertEqual(output.getvalue().count('target_selected'), 1)
        self.assertNotIn('123456', output.getvalue())
        with self.assertRaises(DBusError):
            agent.RequestAuthorization(self.first)
        agent.RequestAuthorization(self.second)

    def test_registered_phone_cannot_take_a_new_device_window(self):
        selection = PeerSelection(self.adapter, pair_seconds=120,
                                  exclude_peers=['00:11:22:33:44:55'])
        agent = PeerAgent(selection)
        with contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertFalse(selection.allows(self.first))
            self.assertFalse(selection.allows(self.first, select=True))
            with self.assertRaises(DBusError):
                agent.RequestAuthorization(self.first)
            self.assertIsNone(selection.selected)
            self.assertTrue(selection.allows(self.second, select=True))
            self.assertTrue(selection.allows(self.second))
        self.assertEqual(selection.selected, self.second)
        self.assertEqual(output.getvalue().count('target_selected'), 1)
        self.assertNotIn('00:11:22:33:44:55', output.getvalue())

    def test_exclusion_normalizes_case_and_explicit_selection_wins(self):
        path = self.adapter + '/dev_AA_BB_CC_DD_EE_FF'
        address = 'aa:bb:cc:dd:ee:ff'
        self.assertEqual(normalize_peer(address), 'AA:BB:CC:DD:EE:FF')
        window = PeerSelection(self.adapter, pair_seconds=5, exclude_peers=[address])
        self.assertFalse(window.allows(path, select=True))
        selected = PeerSelection(self.adapter, selected=path, exclude_peers=[address])
        self.assertTrue(selected.allows(path, select=True))
        self.assertFalse(selected.allows(self.second))
        with self.assertRaises(ValueError):
            PeerSelection(self.adapter, pair_seconds=5, exclude_peers=[address]*9)
        for address in ('', 'aa-bb-cc-dd-ee-ff', 'aa:bb:cc:dd:ee:gg',
                        '  :bb:cc:dd:ee:ff', 'aa:bb:cc:dd:ee', None):
            with self.subTest(address=address), self.assertRaises(ValueError):
                normalize_peer(address)
        with self.assertRaises(ValueError):
            PeerSelection(self.adapter, pair_seconds=5, exclude_peers=['malformed'])

    def test_exclusion_cli_rejects_implicit_window_and_excess_addresses(self):
        cases = [(['--exclude-peer', '00:11:22:33:44:55'], '--exclude-peer requires'),
                 (['--exclude-peer', 'malformed'], '--exclude-peer'),
                 (['--exclude-peer', '00:11:22:33:44:55']*9, 'at most eight')]
        for arguments, expected in cases:
            with self.subTest(arguments=arguments), mock.patch.object(sys, 'argv', ['probe.py', '--model', '00112233', *arguments]):
                with contextlib.redirect_stderr(io.StringIO()) as output, self.assertRaises(SystemExit) as error:
                    main()
                self.assertEqual(error.exception.code, 2)
                self.assertIn(expected, output.getvalue())

    def test_expired_window_cannot_enroll_even_the_previous_candidate(self):
        now = [0.0]
        selection = PeerSelection(self.adapter, pair_seconds=5, clock=lambda: now[0])
        self.assertTrue(selection.allows(self.first))
        now[0] = 5.0
        self.assertFalse(selection.allows(self.first, select=True))
        self.assertIsNone(selection.selected)
        with self.assertRaises(DBusError):
            PeerAgent(selection).RequestAuthorization(self.first)

    def test_selected_phone_can_finish_authentication_after_enrollment_closes(self):
        now = [0.0]
        selection = PeerSelection(self.adapter, pair_seconds=5, clock=lambda: now[0])
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertTrue(selection.allows(self.first, select=True))
        now[0] = 500.0
        self.assertTrue(selection.allows(self.first))
        self.assertFalse(selection.allows(self.second, select=True))

    def test_no_implicit_window_and_validated_adapter_path(self):
        selection = PeerSelection(self.adapter)
        self.assertFalse(selection.allows(self.first, select=True))
        window = PeerSelection(self.adapter, pair_seconds=120)
        for path in ('/org/bluez/hci1/dev_00_11_22_33_44_55',
                     self.first + '/child', self.adapter + '/dev_00_11_ZZ_33_44_55',
                     self.adapter + '/dev_00_11_22_33_44',
                     self.adapter + '/dev_  _11_22_33_44_55', None):
            with self.subTest(path=path):
                self.assertFalse(window.allows(path, select=True))
        self.assertIsNone(window.selected)
        for seconds in (-1, 1, 4, 121):
            with self.assertRaises(ValueError):
                PeerSelection(self.adapter, pair_seconds=seconds)

    def test_explicit_peer_stays_exclusive_without_enrollment_deadline(self):
        selection = PeerSelection(self.adapter, selected=self.first, clock=lambda: 1e20)
        self.assertTrue(selection.allows(self.first, select=True))
        self.assertFalse(selection.allows(self.second, select=True))


class PairProfileTests(unittest.IsolatedAsyncioTestCase):
    @unittest.skipUnless(os.name == 'posix', 'BlueZ transfers POSIX socket file descriptors')
    async def test_profiles_share_selection_and_close_rejected_fds(self):
        first = '/org/bluez/hci0/dev_00_11_22_33_44_55'
        second = '/org/bluez/hci0/dev_00_11_22_33_44_66'
        stop = asyncio.Event()
        selection = PeerSelection('/org/bluez/hci0', pair_seconds=120)
        control = Profile(stop, None, selection=selection)
        business = Profile(stop, None, decode_control=False, selection=selection)
        sockets = [socket.socketpair() for _ in range(3)]
        try:
            with contextlib.redirect_stdout(io.StringIO()):
                control.NewConnection(first, os.dup(sockets[0][0].fileno()), {})
                rejected_fd = os.dup(sockets[1][0].fileno())
                with self.assertRaises(DBusError):
                    business.NewConnection(second, rejected_fd, {})
                with self.assertRaises(OSError):
                    os.fstat(rejected_fd)
                business.NewConnection(first, os.dup(sockets[2][0].fileno()), {})
                self.assertEqual(set(control.tasks), {first})
                self.assertEqual(set(business.tasks), {first})
                await control.close()
                await business.close()
        finally:
            await control.close()
            await business.close()
            for pair in sockets:
                for sock in pair:
                    sock.close()


    @unittest.skipUnless(os.name == 'posix', 'BlueZ transfers POSIX socket file descriptors')
    async def test_excluded_old_phone_fd_closes_then_new_phone_can_connect(self):
        old = '/org/bluez/hci0/dev_00_11_22_33_44_55'
        new = '/org/bluez/hci0/dev_00_11_22_33_44_66'
        selection = PeerSelection('/org/bluez/hci0', pair_seconds=120,
                                  exclude_peers=['00:11:22:33:44:55'])
        stop = asyncio.Event()
        control = Profile(stop, None, selection=selection)
        business = Profile(stop, None, decode_control=False, selection=selection)
        sockets = [socket.socketpair() for _ in range(4)]
        try:
            with contextlib.redirect_stdout(io.StringIO()) as output:
                for profile, pair in zip((control, business), sockets[:2]):
                    descriptor = os.dup(pair[0].fileno())
                    with self.assertRaises(DBusError):
                        profile.NewConnection(old, descriptor, {})
                    with self.assertRaises(OSError):
                        os.fstat(descriptor)
                    self.assertIsNone(selection.selected)
                control.NewConnection(new, os.dup(sockets[2][0].fileno()), {})
                business.NewConnection(new, os.dup(sockets[3][0].fileno()), {})
                self.assertEqual(selection.selected, new)
                self.assertEqual(set(control.tasks), {new})
                self.assertEqual(set(business.tasks), {new})
                await control.close();await business.close()
                self.assertEqual(output.getvalue().count('target_selected'), 1)
        finally:
            await control.close();await business.close()
            for pair in sockets:
                for stream in pair:stream.close()


class StreamTests(unittest.IsolatedAsyncioTestCase):
    async def test_media_finishes_before_underlying_transport_closes(self):
        events = []
        async def control():
            try:
                await asyncio.Future()
            finally:
                events.append('teardown-start')
                await asyncio.sleep(0)
                self.assertNotIn('transport-closed', events)
                events.append('teardown-finished')
        async def transport():
            try:
                await asyncio.Future()
            finally:
                events.append('transport-closed')
        first, second = asyncio.create_task(control()), asyncio.create_task(transport())
        await asyncio.sleep(0)
        await asyncio.wait_for(close_tcp_streams({first}, {first, second}, set()), 1)
        self.assertEqual(events, ['teardown-start', 'teardown-finished', 'transport-closed'])

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
