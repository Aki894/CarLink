"""Clean formal-image checks, without compilers, mounts, identities or devices."""
import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import tempfile
import unittest

spec=importlib.util.spec_from_file_location('production',Path(__file__).with_name('verify-production-runtime.py'))
production=importlib.util.module_from_spec(spec);spec.loader.exec_module(production)

class ProductionTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(prefix='carlink-production-');self.root=Path(self.temp.name)
        self.kernel='6.18.57-current-sunxi';self.native='usr/local/lib/carlink/hicar-native/'
        def elf(hard=True):
            data=bytearray(52);data[:6]=b'\x7fELF\x01\x01';struct.pack_into('<H',data,18,40);struct.pack_into('<I',data,36,0x400 if hard else 0);return bytes(data)
        self.elf=elf
        self.source={'fixture':'native'}
        self.build={'runtime_profile':'production','hicar_native_source':self.source,
                    'hicar_native_sha256':hashlib.sha256(elf()).hexdigest()}
        self.write('etc/carlink-build.json',json.dumps(self.build))
        manager={'schema':1,'hicar':['/usr/local/bin/carlink','hicar','run'],'carlife':['carlink','carlife'],
                 'carplay':['run-carplay.sh'],'menu_library':'menu.so','menu_font':'font.otf'}
        self.write('usr/share/carlink/manager.example.json',json.dumps(manager))
        for name in ('carlink','carlink-run','carlink-carplay'):self.write('usr/local/bin/'+name,elf())
        native_files=['sources.json','libcarlink_hichain.so','lib/libcrypto.so.3','lib/libmbedcrypto.so.7','lib/libcjson.so.1']
        for relative in native_files:self.write(self.native+relative,json.dumps(self.source) if relative=='sources.json' else elf())
        self.rehash_native()
        self.driver={'kernel_release':self.kernel,'driver_commit':'a'*40,'variant':'main',
                     'modules':[f'modules/{self.kernel}/extra/aic8800/{name}.ko' for name in ('aic_load_fw','aic8800_fdrv','aic_zlp_quirk')],
                     'firmware':['firmware/aic8800D80/fw.bin'],'udev':['udev/90-carlink-aic8800.rules']}
        checks={}
        for relative in self.driver['modules']+self.driver['firmware']:
            data=elf(False)+b'vermagic='+self.kernel.encode()+b' SMP\0' if relative.endswith('.ko') else b'firmware fixture'
            self.write('lib/'+relative,data);checks['lib/'+relative]=hashlib.sha256(data).hexdigest()
        udev='etc/udev/rules.d/90-carlink-aic8800.rules';self.write(udev,b'udev fixture');checks[udev]=hashlib.sha256(b'udev fixture').hexdigest()
        self.write('etc/carlink-aic-driver.json',json.dumps(self.driver));self.write('etc/carlink-aic-checksums.json',json.dumps(checks))
        self.write('etc/modprobe.d/carlink-aic.conf','options aic8800_fdrv country_code=CN custregd=0\n')
        self.write('boot/config-'+self.kernel,'CONFIG_USB_CONFIGFS_F_FS=y\n');self.write('usr/sbin/usb_modeswitch',elf())
    def tearDown(self):self.temp.cleanup()
    def write(self,name,data):
        p=self.root/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(data.encode() if isinstance(data,str) else data)
    def rehash_native(self):
        folder=self.root/self.native
        self.write(self.native+'SHA256SUMS',''.join(hashlib.sha256(p.read_bytes()).hexdigest()+'  '+p.relative_to(folder).as_posix()+'\n' for p in sorted(folder.rglob('*')) if p.is_file() and p.name!='SHA256SUMS'))
    def check(self):production.verify(self.root)
    def test_clean_formal_image_requires_no_old_local_state(self):
        self.assertFalse((self.root/'var/lib/carlink').exists());self.check()
        self.assertFalse((self.root/'var/lib/carlink').exists())
    def test_manager_example_is_required(self):
        (self.root/'usr/share/carlink/manager.example.json').unlink()
        with self.assertRaisesRegex(ValueError,'Missing production payload'):self.check()
    def test_baseline_cannot_be_released_as_production(self):
        self.build['runtime_profile']='baseline';self.write('etc/carlink-build.json',json.dumps(self.build))
        with self.assertRaisesRegex(ValueError,'not a formal'):self.check()
    def test_native_required_even_if_manifest_omits_it(self):
        self.build.pop('hicar_native_source');self.write('etc/carlink-build.json',json.dumps(self.build))
        with self.assertRaisesRegex(ValueError,'native source is missing'):self.check()
    def test_missing_native_dependency_is_rejected(self):
        (self.root/(self.native+'lib/libcrypto.so.3')).unlink()
        with self.assertRaises(ValueError):self.check()
    def test_wrong_native_abi_rejected_after_valid_manifest(self):
        self.write(self.native+'lib/libcrypto.so.3',self.elf(False));self.rehash_native()
        with self.assertRaisesRegex(ValueError,'hard-float'):self.check()
    def test_missing_aic_module_is_rejected(self):
        (self.root/('lib/'+self.driver['modules'][0])).unlink()
        with self.assertRaisesRegex(ValueError,'Missing production payload'):self.check()
    def test_legacy_radio_without_aic_receipt_is_rejected(self):
        (self.root/'etc/carlink-aic-driver.json').unlink()
        with self.assertRaisesRegex(ValueError,'Missing production payload'):self.check()
    def test_aic_firmware_tamper_is_rejected(self):
        self.write('lib/'+self.driver['firmware'][0],b'wrong firmware')
        with self.assertRaisesRegex(ValueError,'AIC checksum differs'):self.check()
    def test_wrong_kernel_module_rejected_even_with_valid_checksum(self):
        relative='lib/'+self.driver['modules'][0];data=self.elf(False)+b'vermagic=wrong-kernel SMP\0';self.write(relative,data)
        checks=json.loads((self.root/'etc/carlink-aic-checksums.json').read_text());checks[relative]=hashlib.sha256(data).hexdigest();self.write('etc/carlink-aic-checksums.json',json.dumps(checks))
        with self.assertRaisesRegex(ValueError,'ABI/kernel differs'):self.check()
    def test_outside_payload_path_is_rejected(self):
        checks=json.loads((self.root/'etc/carlink-aic-checksums.json').read_text());checks['../outside']='a'*64;self.write('etc/carlink-aic-checksums.json',json.dumps(checks))
        with self.assertRaisesRegex(ValueError,'payload manifest differs'):self.check()

if __name__=='__main__':unittest.main()
