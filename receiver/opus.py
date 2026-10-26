"""Small libopus binding for 20 ms microphone packets; no device or file capture."""
import ctypes as C
import ctypes.util
import os
from pathlib import Path


class Encoder:
    def __init__(self, channels):
        root = Path(__file__).resolve().parent
        local = next((p for p in (root/'lib/libcarlink_opus.so', root.parent.parent/'lib/libcarlink_opus.so') if p.is_file()), None)
        path = os.environ.get('CARLINK_OPUS_LIBRARY') or (str(local) if local else C.util.find_library('opus'))
        if not path:
            raise ValueError('microphone Opus library unavailable')
        self.lib = C.CDLL(path)
        self.lib.opus_encoder_create.argtypes = [C.c_int, C.c_int, C.c_int, C.POINTER(C.c_int)]
        self.lib.opus_encoder_create.restype = C.c_void_p
        self.lib.opus_encoder_destroy.argtypes = [C.c_void_p]
        self.lib.opus_encode.argtypes = [C.c_void_p, C.POINTER(C.c_int16), C.c_int, C.POINTER(C.c_ubyte), C.c_int32]
        self.lib.opus_encode.restype = C.c_int
        self.lib.opus_encoder_ctl.argtypes = [C.c_void_p, C.c_int, C.c_int]
        error = C.c_int()
        self.handle = self.lib.opus_encoder_create(48000, channels, 2048, C.byref(error))
        self.channels = channels
        if not self.handle or error.value:
            self.close()
            raise ValueError('microphone Opus encoder creation failed')
        for option, value in ((4002, 32000 * channels), (4010, 3)):
            if self.lib.opus_encoder_ctl(self.handle, option, value):
                self.close()
                raise ValueError('microphone Opus encoder configuration failed')

    def encode(self, mono):
        if not self.handle or len(mono) != 1920:
            raise ValueError('Opus microphone requires 960 mono samples')
        pcm = mono if self.channels == 1 else b''.join(mono[i:i+2] * 2 for i in range(0, len(mono), 2))
        samples = (C.c_int16 * (960 * self.channels)).from_buffer_copy(pcm)
        output = (C.c_ubyte * 1275)()
        size = self.lib.opus_encode(self.handle, samples, 960, output, len(output))
        if size <= 0:
            raise ValueError('microphone Opus encoding failed')
        return bytes(output[:size])

    def close(self):
        if getattr(self, 'handle', None):
            self.lib.opus_encoder_destroy(self.handle)
            self.handle = None

    def __del__(self):
        self.close()
