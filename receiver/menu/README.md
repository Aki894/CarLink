# Local device menu renderer

`menu.c` owns only the 800×480 local CarLink menu. It does not decode, buffer or
encode either phone's projection stream. Every changed menu view is an Annex-B
SPS/PPS/IDR frame; a static view is repeated at one frame per second for recovery.
The Rust mux forwards source PCM while the menu is visible and requests a fresh
source keyframe when returning to projection.

CI downloads the exact minih264, stb_truetype and Noto CJK font files identified
in `sources.lock.json`, verifies their SHA-256 hashes, builds host and ARMv7
helpers and decodes two native test frames. The packaged source and license
files identify minih264's CC0 dedication, stb's license and the font's OFL.
The installed shared library and font are trusted local assets; do not load
fonts supplied by paired phones or the management web UI.

C ABI:
- `carlink_menu_create(const char *font_path) -> void *` (NULL on failure)
- `carlink_menu_render(void *, uint32_t **rows, uint32_t *lengths, uint32_t count,
  uint32_t focus, unsigned char *output, uint32_t capacity) -> int` (bytes or -1)
- `carlink_menu_destroy(void *)`

Rows are UTF-32 scalar values supplied by the Rust owner. At most 24 rows and
128 scalars per row are accepted. Seven rows are visible; focus scrolls them.
The output buffer is at least 512 KiB. Context and returned encoder buffers
remain confined to the mux thread.
