import array
import ctypes.util
import math
import shutil
import struct
import subprocess
import unittest

import forward
import opus


class OpusTests(unittest.TestCase):
    @unittest.skipUnless(ctypes.util.find_library('opus') and shutil.which('ffmpeg'), 'host libopus and FFmpeg required')
    def test_real_encoder_20ms_mono_and_stereo_roundtrip(self):
        mono = array.array('h', (int(9000 * math.sin(i * 2 * math.pi * 440 / 48000)) for i in range(960))).tobytes()
        for channels in (1, 2):
            encoder = opus.Encoder(channels)
            packets = [encoder.encode(mono) for _ in range(8)]
            encoder.close()
            self.assertTrue(all(forward.opus_samples(p) == 960 for p in packets))
            head = b'OpusHead' + struct.pack('<BBHIhB', 1, channels, 0, 48000, 0, 0)
            tags = b'OpusTags' + struct.pack('<I', 7) + b'CarLink' + struct.pack('<I', 0)
            ogg = forward.ogg_page(head, 0, 0, True) + forward.ogg_page(tags, 1, 0)
            ogg += b''.join(forward.ogg_page(p, i+2, (i+1)*960) for i,p in enumerate(packets))
            result = subprocess.run(['ffmpeg','-hide_banner','-loglevel','error','-f','ogg','-i','pipe:0',
                '-f','s16le','-ac','1','-ar','48000','pipe:1'],input=ogg,capture_output=True,check=True,timeout=10)
            decoded = array.array('h',result.stdout)
            self.assertGreater(len(decoded), 6000)
            self.assertGreater(max(map(abs,decoded)), 5000)


if __name__ == '__main__':
    unittest.main()
