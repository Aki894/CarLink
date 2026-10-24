"""Exercise image installation with actual downloaded CI artifacts and corruptions."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import unittest

base = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('runtime_install', base / 'scripts/install-projection-runtime.py')
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)
app_source, receiver_source = (Path(p).resolve() for p in sys.argv[1:3])


class InstallTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='carlink-install-')
        self.root = Path(self.temp.name).resolve()
        self.assertEqual(self.root.parent, Path(tempfile.gettempdir()).resolve())
        # All recursive cleanup/copy targets are owned by this new temporary directory.
        self.app, self.receiver, self.project, self.image = [self.root / p for p in ('app', 'receiver', 'project', 'image')]
        shutil.copytree(app_source, self.app)
        shutil.copytree(receiver_source, self.receiver)
        self.project.mkdir()
        shutil.copyfile(base / 'sources.lock.json', self.project / 'sources.lock.json')
        shutil.copytree(self.receiver / 'share/source/patches', self.project / 'receiver/patches')
        (self.image / 'etc').mkdir(parents=True)
        (self.image / 'etc/carlink-build.json').write_text('{}')
        source = json.loads((self.receiver / 'SOURCE.json').read_text())
        os.environ.update(APP_COMMIT=source['project_commit'], APP_RUN_ID='1', REFRESH_COMMIT='0' * 40)

    def tearDown(self):
        self.assertEqual(Path(self.temp.name).resolve(), self.root)
        self.temp.cleanup()

    def install(self):
        installer.install(self.project, self.app, self.receiver, self.image)

    def test_valid_install_preserves_local_identity_boundary(self):
        self.install()
        manifest = json.loads((self.image / 'etc/carlink-build.json').read_text())
        self.assertEqual(manifest['app_sha256'], hashlib.sha256((self.app / 'carlink').read_bytes()).hexdigest())
        self.assertTrue((self.image / 'usr/local/bin/carlink-carplay').is_file())
        self.assertEqual(list((self.image / 'var/lib/carlink/carplay/mfi').iterdir()), [])
        if (self.app/'share').is_dir():
            self.assertTrue(manifest['app_notices'])
            self.assertTrue((self.image/'usr/local/lib/carlink/app/share/licenses/rust/SOURCE.json').is_file())

    def test_corrupt_receiver_rejected_before_install(self):
        (self.receiver / 'probe.py').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
            self.install()
        self.assertFalse((self.image / 'usr').exists())

    def test_extra_identity_rejected(self):
        (self.receiver / 'identity.pk8').write_bytes(b'fixture')
        with self.assertRaisesRegex(ValueError, 'Private runtime'):
            self.install()

    def test_stale_patch_rejected(self):
        (self.project / 'receiver/patches/stale.patch').write_text('wrong revision')
        with self.assertRaisesRegex(ValueError, 'patches do not match'):
            self.install()

    def test_checksum_traversal_rejected(self):
        with (self.receiver / 'SHA256SUMS').open('a') as f:
            f.write('0' * 64 + '  ../outside\n')
        with self.assertRaisesRegex(ValueError, 'Unsafe'):
            self.install()


unittest.main(argv=[sys.argv[0]])
