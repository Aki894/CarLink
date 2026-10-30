"""Offline filesystem policy tests; never boots or accesses a board."""
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
import shutil
from unittest import mock

spec = importlib.util.spec_from_file_location('boot_profile', Path(__file__).with_name('install-boot-profile.py'))
profile = importlib.util.module_from_spec(spec)
spec.loader.exec_module(profile)


class BootProfileTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='carlink-boot-profile-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / 'root'
        (self.root / 'etc/systemd/system').mkdir(parents=True)

    def fixture(self, version):
        target = Path(self.temp.name) / ('profile-' + str(version))
        shutil.copytree(profile.PROFILE, target)
        policy = json.loads((target / 'policy.json').read_text())
        policy['version'] = version
        allowed = profile.V1_PATHS if version == 1 else profile.VERSION_PATHS[2]
        for path in (target / 'rootfs').rglob('*'):
            if path.is_file() and path.relative_to(target / 'rootfs').as_posix() not in allowed:
                path.unlink()
        policy['masks'] = {name: reason for name, reason in policy['masks'].items()
                           if 'etc/systemd/system/' + name in allowed}
        if version == 2:
            for name in profile.V2_ADDITIONS:
                if name.endswith('.service') or name.endswith('.socket'):
                    policy['masks'][name.rsplit('/', 1)[-1]] = 'headless fixture'
            timer = target / 'rootfs/etc/systemd/system/logrotate.timer.d/80-carlink.conf'
            timer.parent.mkdir(parents=True, exist_ok=True)
            timer.write_text('[Timer]\nOnBootSec=10min\n')
        (target / 'policy.json').write_text(json.dumps(policy))
        return target

    def test_legacy_upgrade_preserves_first_backup_and_reverts_all_paths(self):
        old, new = self.fixture(1), self.fixture(2)
        unit = self.root / 'etc/systemd/system/carlink.service'
        unit.write_text('before first install')
        unit.chmod(0o640)
        syslog = self.root / 'etc/systemd/system/syslog.socket'
        syslog.symlink_to('/lib/systemd/system/syslog.socket')
        profile.operate(self.root, profile=old)
        manifest = self.root / profile.BACKUP
        record = json.loads(manifest.read_text())
        # Original deployed manifests predate an explicit profile_version.
        record.pop('profile_version')
        manifest.write_text(json.dumps(record))
        source = new / 'rootfs/etc/systemd/system/carlink.service'
        source.write_text(source.read_text() + '\n# upgraded content\n')
        with self.assertRaisesRegex(ValueError, 'upgrade required'):
            profile.operate(self.root, 'verify', new)
        self.assertEqual(profile.operate(self.root, profile=new)['action'], 'upgrade')
        self.assertEqual(unit.read_text(), source.read_text())
        self.assertEqual(syslog.readlink(), Path('/dev/null'))
        self.assertEqual(profile.operate(self.root, profile=new)['action'], 'unchanged')
        profile.operate(self.root, 'verify', new)
        self.assertEqual(json.loads(manifest.read_text())['entries']['etc/systemd/system/carlink.service']['before'], record['entries']['etc/systemd/system/carlink.service']['before'])
        with self.assertRaisesRegex(ValueError, 'downgrade'):
            profile.operate(self.root, profile=old)
        # Even an older profile supplied to the new installer reverts every
        # explicitly recognized path owned by the installed newer version.
        result = profile.operate(self.root, 'revert', old)
        self.assertEqual(result['files'], len(profile.VERSION_PATHS[2]))
        self.assertEqual(unit.read_text(), 'before first install')
        if os.name != 'nt':
            self.assertEqual(unit.stat().st_mode & 0o777, 0o640)
        self.assertEqual(syslog.readlink(), Path('/lib/systemd/system/syslog.socket'))
        self.assertFalse((self.root / 'etc/systemd/system/logrotate.timer.d/80-carlink.conf').exists())
        self.assertFalse(manifest.exists())

    def test_upgrade_rejects_administrator_changes_before_mutation(self):
        old, new = self.fixture(1), self.fixture(2)
        profile.operate(self.root, profile=old)
        unit = self.root / 'etc/systemd/system/carlink.service'
        unit.write_text('administrator override')
        manifest = self.root / profile.BACKUP
        original = manifest.read_bytes()
        with self.assertRaisesRegex(ValueError, 'changed since installation'):
            profile.operate(self.root, profile=new)
        self.assertEqual(manifest.read_bytes(), original)
        self.assertEqual(unit.read_text(), 'administrator override')
        self.assertFalse((self.root / 'etc/systemd/system/syslog.socket').is_symlink())

    def test_deployed_thirteen_path_profile_preserves_independent_journal_dropin(self):
        old, new = self.fixture(1), self.fixture(2)
        profile.operate(self.root, profile=old)
        manifest = self.root / profile.BACKUP
        record = json.loads(manifest.read_text())
        record.pop('profile_version')
        journal = 'etc/systemd/system/carlink.service.d/80-journal.conf'
        record['entries'].pop(journal)
        self.assertEqual(set(record['entries']), profile.LEGACY_V1_PATHS)
        manifest.write_text(json.dumps(record))
        # The actual device had this file added outside the older manifest.
        existing = self.root / journal
        existing.write_text('independently installed journal configuration\n')
        prior = profile.snapshot(existing)
        self.assertEqual(profile.operate(self.root, profile=new)['action'], 'upgrade')
        self.assertEqual(json.loads(manifest.read_text())['entries'][journal]['before'], prior)
        profile.operate(self.root, 'verify', new)
        profile.operate(self.root, 'revert', new)
        self.assertEqual(profile.snapshot(existing), prior)

    def test_other_thirteen_path_subset_is_not_a_known_legacy_profile(self):
        old, new = self.fixture(1), self.fixture(2)
        profile.operate(self.root, profile=old)
        manifest = self.root / profile.BACKUP
        record = json.loads(manifest.read_text())
        record['entries'].pop('etc/systemd/system/default.target')
        manifest.write_text(json.dumps(record))
        with self.assertRaisesRegex(ValueError, 'allowlist'):
            profile.operate(self.root, profile=new)

    def test_upgrade_rejects_unrecognized_historical_path(self):
        old, new = self.fixture(1), self.fixture(2)
        profile.operate(self.root, profile=old)
        manifest = self.root / profile.BACKUP
        record = json.loads(manifest.read_text())
        record['entries']['etc/hosts'] = {'before': {'kind': 'absent'}, 'after': {'kind': 'absent'}}
        manifest.write_text(json.dumps(record))
        original = manifest.read_bytes()
        with self.assertRaisesRegex(ValueError, 'allowlist'):
            profile.operate(self.root, profile=new)
        with self.assertRaisesRegex(ValueError, 'allowlist'):
            profile.operate(self.root, 'revert', new)
        self.assertEqual(manifest.read_bytes(), original)
        self.assertFalse((self.root / 'etc/hosts').exists())

    def test_upgrade_rejects_new_linked_parent(self):
        old, new = self.fixture(1), self.fixture(2)
        profile.operate(self.root, profile=old)
        outside = Path(self.temp.name) / 'new-outside'
        outside.mkdir()
        (self.root / 'etc/systemd/system/logrotate.timer.d').symlink_to(outside, target_is_directory=True)
        manifest = self.root / profile.BACKUP
        original = manifest.read_bytes()
        with self.assertRaises(ValueError):
            profile.operate(self.root, profile=new)
        self.assertEqual(manifest.read_bytes(), original)
        self.assertEqual(list(outside.iterdir()), [])

    def test_upgrade_failure_restores_previous_install_and_manifest(self):
        old, new = self.fixture(1), self.fixture(2)
        profile.operate(self.root, profile=old)
        manifest = self.root / profile.BACKUP
        previous_record = json.loads(manifest.read_text())
        previous_files = {name: profile.snapshot(self.root / name) for name in profile.VERSION_PATHS[2]}
        real_write = profile.write
        calls = 0

        def fail_once(path, state):
            nonlocal calls
            calls += 1
            if calls == 4:
                raise OSError('injected write failure')
            return real_write(path, state)

        with mock.patch.object(profile, 'write', side_effect=fail_once):
            with self.assertRaisesRegex(OSError, 'injected'):
                profile.operate(self.root, profile=new)
        self.assertEqual(json.loads(manifest.read_text()), previous_record)
        for name, state in previous_files.items():
            self.assertEqual(profile.snapshot(self.root / name), state)
        profile.operate(self.root, 'verify', old)

    def test_reject_unknown_profile_version_before_changes(self):
        new = self.fixture(2)
        policy = json.loads((new / 'policy.json').read_text())
        policy['version'] = 3
        (new / 'policy.json').write_text(json.dumps(policy))
        with self.assertRaisesRegex(ValueError, 'Unknown boot profile'):
            profile.operate(self.root, profile=new)
        self.assertFalse((self.root / profile.BACKUP).exists())

    def test_install_verify_idempotence(self):
        result = profile.operate(self.root)
        self.assertGreater(result['files'], 10)
        self.assertEqual(profile.operate(self.root)['action'], 'unchanged')
        profile.operate(self.root, 'verify')
        self.assertEqual((self.root / 'etc/systemd/system/NetworkManager-wait-online.service').readlink(), Path('/dev/null'))

    def test_preserve_management_and_revert(self):
        system = self.root / 'etc/systemd/system'
        policy = json.loads((profile.PROFILE / 'policy.json').read_text())
        for name in policy['preserved'] + ['serial-getty@ttyS0.service']:
            (system / name).write_text('existing-' + name)
        legacy = system / 'getty.target.wants/serial-getty@ttyGS0.service'
        legacy.parent.mkdir()
        legacy.symlink_to('/lib/systemd/system/serial-getty@.service')
        replaced = system / 'ModemManager.service'
        replaced.write_bytes(b'original modem unit\n')
        profile.operate(self.root)
        self.assertFalse(legacy.is_symlink())
        for name in policy['preserved'] + ['serial-getty@ttyS0.service']:
            self.assertEqual((system / name).read_text(), 'existing-' + name)
        profile.operate(self.root, 'revert')
        self.assertEqual(replaced.read_bytes(), b'original modem unit\n')
        self.assertEqual(legacy.readlink(), Path('/lib/systemd/system/serial-getty@.service'))
        self.assertFalse((system / 'carlink.service').exists())

    def test_service_has_local_dependencies_without_global_protocol_guards(self):
        profile.operate(self.root)
        unit = (self.root / 'etc/systemd/system/carlink.service').read_text()
        self.assertIn('After=NetworkManager.service bluetooth.service', unit)
        self.assertNotIn('network-online.target', unit)
        self.assertIn('RestartSec=5s', unit)
        self.assertIn('RuntimeDirectoryMode=0700', unit)
        self.assertNotIn('ConditionPathExists=', unit)
        runner = (profile.PROFILE.parent.parent / 'scripts/run-carplay.sh').read_text()
        self.assertIn('identity.pk8 certificate.p7b', runner)
        logging = (self.root / 'etc/systemd/system/carlink.service.d/80-journal.conf').read_text()
        self.assertIn('ExecStartPre=/usr/bin/journalctl --flush', logging)
        starts = [line for line in logging.splitlines() if line.startswith('ExecStartPre=')]
        self.assertEqual(starts, ['ExecStartPre=/usr/bin/journalctl --flush'])
        journal = (self.root / 'etc/systemd/journald.conf.d/80-carlink.conf').read_text()
        self.assertIn('Storage=persistent', journal)
        self.assertIn('SyncIntervalSec=5s', journal)

    def test_updates_are_delayed_not_masked(self):
        profile.operate(self.root)
        system = self.root / 'etc/systemd/system'
        for unit in ['apt-daily', 'apt-daily-upgrade']:
            self.assertFalse((system / (unit + '.timer')).is_symlink())
            text = (system / (unit + '.timer.d/80-carlink.conf')).read_text()
            self.assertIn('OnUnitActiveSec=1day', text)
            self.assertIn('OnBootSec=', text)
        self.assertIn('SystemMaxUse=16M', (self.root / 'etc/systemd/journald.conf.d/80-carlink.conf').read_text())

    def test_headless_profile_keeps_serial_recovery_and_native_logging(self):
        profile.operate(self.root)
        system = self.root / 'etc/systemd/system'
        for unit in ('rsyslog.service', 'syslog.socket', 'getty@tty1.service',
                     'keyboard-setup.service', 'console-setup.service'):
            self.assertEqual((system / unit).readlink(), Path('/dev/null'))
        for unit in ('serial-getty@ttyS0.service', 'ssh.service',
                     'systemd-journald.service', 'systemd-journald-dev-log.socket',
                     'armbian-zram-config.service', 'avahi-daemon.service'):
            self.assertFalse((system / unit).is_symlink())

    def test_log_rotation_does_not_catch_up_during_startup(self):
        profile.operate(self.root)
        timer = (self.root / 'etc/systemd/system/logrotate.timer.d/80-carlink.conf').read_text()
        self.assertIn('OnCalendar=\n', timer)
        self.assertIn('OnBootSec=20min', timer)
        self.assertIn('OnUnitActiveSec=1day', timer)
        self.assertIn('Persistent=false', timer)

    def test_reject_host_root(self):
        with self.assertRaises(ValueError):
            profile.operate(Path(self.root.anchor))

    def test_reject_linked_parent_before_mutation(self):
        outside = Path(self.temp.name) / 'outside'
        outside.mkdir()
        (self.root / 'etc/systemd/journald.conf.d').symlink_to(outside, target_is_directory=True)
        with self.assertRaises(ValueError):
            profile.operate(self.root)
        self.assertEqual(list(outside.iterdir()), [])
        self.assertFalse((self.root / profile.BACKUP).exists())

    def test_reject_linked_backup(self):
        backup = self.root / profile.BACKUP
        backup.parent.mkdir()
        backup.symlink_to(Path(self.temp.name) / 'outside-backup')
        with self.assertRaises(ValueError):
            profile.operate(self.root)

    def test_reject_edited_configuration_on_revert(self):
        profile.operate(self.root)
        unit = self.root / 'etc/systemd/system/carlink.service'
        unit.write_text('administrator change')
        with self.assertRaises(ValueError):
            profile.operate(self.root, 'revert')
        self.assertEqual(unit.read_text(), 'administrator change')

    def test_reject_manifest_path_injection(self):
        profile.operate(self.root)
        backup = self.root / profile.BACKUP
        record = json.loads(backup.read_text())
        record['entries']['../../escape'] = {'before': {'kind': 'absent'}, 'after': {'kind': 'absent'}}
        backup.write_text(json.dumps(record))
        with self.assertRaises(ValueError):
            profile.operate(self.root, 'revert')


if __name__ == '__main__':
    unittest.main()
