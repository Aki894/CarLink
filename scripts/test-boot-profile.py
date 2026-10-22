"""Offline filesystem policy tests; never boots or accesses a board."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('boot_profile', Path(__file__).with_name('install-boot-profile.py'))
profile = importlib.util.module_from_spec(spec)
spec.loader.exec_module(profile)


class BootProfileTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='carlink-boot-profile-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / 'root'
        (self.root / 'etc/systemd/system').mkdir(parents=True)

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

    def test_service_has_local_dependencies_and_identity_guards(self):
        profile.operate(self.root)
        unit = (self.root / 'etc/systemd/system/carlink.service').read_text()
        self.assertIn('After=NetworkManager.service bluetooth.service', unit)
        self.assertNotIn('network-online.target', unit)
        self.assertIn('RestartSec=5s', unit)
        self.assertIn('RuntimeDirectoryMode=0700', unit)
        self.assertEqual(unit.count('ConditionPathExists='), 2)
        logging = (self.root / 'etc/systemd/system/carlink.service.d/80-journal.conf').read_text()
        self.assertIn('ExecStartPre=/usr/bin/journalctl --flush', logging)
        self.assertIn('ExecStartPre=/usr/bin/journalctl --sync', logging)
        journal = (self.root / 'etc/systemd/journald.conf.d/80-carlink.conf').read_text()
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
