"""Exercise image installation with actual downloaded CI artifacts and corruptions."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import sys
import tempfile
import unittest

base = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('runtime_install', base / 'scripts/install-projection-runtime.py')
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)
app_source, receiver_source = (Path(p).resolve() for p in sys.argv[1:3])
native_source = Path(sys.argv[3]).resolve() if len(sys.argv) > 3 else None


class InstallTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='carlink-install-')
        self.root = Path(self.temp.name).resolve()
        self.assertEqual(self.root.parent, Path(tempfile.gettempdir()).resolve())
        # All recursive cleanup/copy targets are owned by this new temporary directory.
        self.app, self.receiver, self.project, self.image = [self.root / p for p in ('app', 'receiver', 'project', 'image')]
        shutil.copytree(app_source, self.app)
        shutil.copytree(receiver_source, self.receiver)
        self.previous_native = os.environ.get('HICAR_NATIVE_DIR')
        self.native = None
        if native_source is not None:
            self.native = self.root / 'native'
            shutil.copytree(native_source, self.native)
            os.environ['HICAR_NATIVE_DIR'] = str(self.native)
        else:
            # This invocation intentionally exercises the optional legacy path.
            os.environ.pop('HICAR_NATIVE_DIR', None)
        self.project.mkdir()
        shutil.copyfile(base / 'sources.lock.json', self.project / 'sources.lock.json')
        example = self.project / 'docs/examples/manager.json'
        example.parent.mkdir(parents=True)
        shutil.copyfile(base / 'docs/examples/manager.json', example)
        native_lock = self.project / 'experiments/hicar/native/sources.json'
        native_lock.parent.mkdir(parents=True)
        shutil.copyfile(base / 'experiments/hicar/native/sources.json', native_lock)
        shutil.copytree(self.receiver / 'share/source/patches', self.project / 'receiver/patches')
        (self.image / 'etc').mkdir(parents=True)
        (self.image / 'etc/carlink-build.json').write_text('{}')
        source = json.loads((self.receiver / 'SOURCE.json').read_text())
        os.environ.update(APP_COMMIT=source['project_commit'], APP_RUN_ID='1', REFRESH_COMMIT='0' * 40)

    def tearDown(self):
        self.assertEqual(Path(self.temp.name).resolve(), self.root)
        self.temp.cleanup()
        if self.previous_native is None:
            os.environ.pop('HICAR_NATIVE_DIR', None)
        else:
            os.environ['HICAR_NATIVE_DIR'] = self.previous_native

    def install(self):
        installer.install(self.project, self.app, self.receiver, self.image)

    def test_missing_manager_example_rejected_before_install(self):
        (self.project / 'docs/examples/manager.json').unlink()
        with self.assertRaisesRegex(ValueError, 'Missing manager runtime example'):
            self.install()
        self.assertFalse((self.image / 'usr').exists())

    def test_valid_install_preserves_local_identity_boundary(self):
        self.install()
        manifest = json.loads((self.image / 'etc/carlink-build.json').read_text())
        self.assertEqual(manifest['app_sha256'], hashlib.sha256((self.app / 'carlink').read_bytes()).hexdigest())
        self.assertTrue((self.image / 'usr/local/bin/carlink-carplay').is_file())
        self.assertEqual(json.loads((self.image / 'usr/share/carlink/manager.example.json').read_text()),
                         json.loads((self.project / 'docs/examples/manager.json').read_text()))
        self.assertEqual(manifest['runtime_profile'], 'production' if self.native else 'baseline')
        runtime = self.image / 'usr/local/lib/carlink/carplay'
        installer.armhf(runtime / 'lib/libcarlink_opus.so')
        self.assertTrue((runtime / 'opus.py').is_file())
        self.assertTrue((runtime / 'share/licenses/opus/COPYING').is_file())
        self.assertEqual(list((self.image / 'var/lib/carlink/carplay/mfi').iterdir()), [])
        if (self.app/'share').is_dir():
            self.assertTrue(manifest['app_notices'])
            self.assertTrue((self.image/'usr/local/lib/carlink/app/share/licenses/rust/SOURCE.json').is_file())

    @unittest.skipUnless(native_source is not None, 'no optional HiCar native artifact supplied')
    def test_native_install_manifest_and_permissions(self):
        self.install()
        destination = self.image / 'usr/local/lib/carlink/hicar-native'
        manifest = json.loads((self.image / 'etc/carlink-build.json').read_text())
        expected_source = json.loads((self.project / 'experiments/hicar/native/sources.json').read_text())
        self.assertEqual(manifest['hicar_native_source'], expected_source)
        self.assertEqual(manifest['hicar_native_sha256'], hashlib.sha256((self.native / 'libcarlink_hichain.so').read_bytes()).hexdigest())
        self.assertEqual(installer.checksums(destination), installer.checksums(self.native))
        for relative in ('libcarlink_hichain.so', 'lib/libcrypto.so.3', 'lib/libmbedcrypto.so.7', 'lib/libcjson.so.1'):
            installer.armhf(destination / relative)
        for path in destination.rglob('*'):
            self.assertEqual(path.stat().st_mode & 0o777, 0o755 if path.is_dir() else 0o644)
        self.assertEqual(list((self.image / 'var/lib/carlink/carplay/mfi').iterdir()), [])

    def rehash_native(self, relative):
        lines = (self.native / 'SHA256SUMS').read_text().splitlines()
        digest = hashlib.sha256((self.native / relative).read_bytes()).hexdigest()
        lines = [digest + '  ' + name if PurePosixPath(name).as_posix() == relative else old + '  ' + name
                 for old, name in (line.split('  ', 1) for line in lines)]
        (self.native / 'SHA256SUMS').write_text('\n'.join(lines) + '\n')

    @unittest.skipUnless(native_source is not None, 'no optional HiCar native artifact supplied')
    def test_corrupt_native_checksum_rejected_before_install(self):
        with (self.native / 'libcarlink_hichain.so').open('ab') as f:
            f.write(b'corrupt fixture')
        with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
            self.install()
        self.assertFalse((self.image / 'usr').exists())

    @unittest.skipUnless(native_source is not None, 'no optional HiCar native artifact supplied')
    def test_native_duplicate_canonical_manifest_rejected(self):
        relative = 'sources.json'
        digest = hashlib.sha256((self.native / relative).read_bytes()).hexdigest()
        lines = (self.native / 'SHA256SUMS').read_text().splitlines()
        # Both spellings address the same file; they must not bypass uniqueness.
        lines = [line for line in lines if PurePosixPath(line.split('  ', 1)[1]).as_posix() != relative]
        lines.extend([digest + '  sources.json', digest + '  ./sources.json'])
        (self.native / 'SHA256SUMS').write_text('\n'.join(lines) + '\n')
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            self.install()
        self.assertFalse((self.image / 'usr').exists())

    @unittest.skipUnless(native_source is not None, 'no optional HiCar native artifact supplied')
    def test_native_source_mismatch_rejected_after_valid_checksum(self):
        source = json.loads((self.native / 'sources.json').read_text())
        source['deviceauth']['commit'] = '0' * 40
        (self.native / 'sources.json').write_text(json.dumps(source))
        self.rehash_native('sources.json')
        with self.assertRaisesRegex(ValueError, 'HiCar native source mismatch'):
            self.install()
        self.assertFalse((self.image / 'usr').exists())

    @unittest.skipUnless(native_source is not None, 'no optional HiCar native artifact supplied')
    def test_missing_native_dependency_rejected_with_consistent_manifest(self):
        relative = 'lib/libcrypto.so.3'
        (self.native / relative).unlink()
        lines = (self.native / 'SHA256SUMS').read_text().splitlines()
        (self.native / 'SHA256SUMS').write_text('\n'.join(line for line in lines if PurePosixPath(line.split('  ', 1)[1]).as_posix() != relative) + '\n')
        with self.assertRaisesRegex(ValueError, 'Missing HiCar native runtime'):
            self.install()
        self.assertFalse((self.image / 'usr').exists())

    @unittest.skipUnless(native_source is not None, 'no optional HiCar native artifact supplied')
    def test_native_soft_float_rejected_after_valid_checksum(self):
        path = self.native / 'lib/libcjson.so.1'
        data = bytearray(path.read_bytes())
        flags = int.from_bytes(data[36:40], 'little') & ~0x400
        data[36:40] = flags.to_bytes(4, 'little')
        path.write_bytes(data)
        self.rehash_native('lib/libcjson.so.1')
        with self.assertRaisesRegex(ValueError, 'ARM hard-float'):
            self.install()
        self.assertFalse((self.image / 'usr').exists())

    def test_corrupt_receiver_rejected_before_install(self):
        (self.receiver / 'probe.py').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
            self.install()
        self.assertFalse((self.image / 'usr').exists())

    def rehash_receiver(self, relative):
        lines = (self.receiver / 'SHA256SUMS').read_text().splitlines()
        digest = hashlib.sha256((self.receiver / relative).read_bytes()).hexdigest()
        lines = [digest + '  ' + name if PurePosixPath(name).as_posix() == relative else old + '  ' + name
                 for old, name in (line.split('  ', 1) for line in lines)]
        (self.receiver / 'SHA256SUMS').write_text('\n'.join(lines) + '\n')

    def test_opus_source_mismatch_rejected_after_valid_checksum(self):
        relative = 'share/source/opus/SOURCE.json'
        source = json.loads((self.receiver / relative).read_text())
        source['commit'] = '0' * 40
        (self.receiver / relative).write_text(json.dumps(source))
        self.rehash_receiver(relative)
        with self.assertRaisesRegex(ValueError, 'Opus microphone source lock mismatch'):
            self.install()
        self.assertFalse((self.image / 'usr').exists())

    def test_opus_soft_float_rejected_after_valid_checksum(self):
        relative = 'lib/libcarlink_opus.so'
        path = self.receiver / relative
        data = bytearray(path.read_bytes())
        flags = int.from_bytes(data[36:40], 'little') & ~0x400
        data[36:40] = flags.to_bytes(4, 'little')
        path.write_bytes(data)
        self.rehash_receiver(relative)
        with self.assertRaisesRegex(ValueError, 'ARM hard-float'):
            self.install()
        self.assertFalse((self.image / 'usr').exists())

    def test_missing_opus_runtime_rejected_with_consistent_manifest(self):
        relative = 'lib/libcarlink_opus.so'
        (self.receiver / relative).unlink()
        lines = (self.receiver / 'SHA256SUMS').read_text().splitlines()
        (self.receiver / 'SHA256SUMS').write_text('\n'.join(line for line in lines
            if PurePosixPath(line.split('  ', 1)[1]).as_posix() != relative) + '\n')
        with self.assertRaisesRegex(ValueError, 'Missing receiver payload'):
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
