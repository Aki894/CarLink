// SPDX-License-Identifier: MIT
// CarLink's local menu only. Phone video is never decoded or encoded here.
#define _POSIX_C_SOURCE 200809L
#include <assert.h>
#include <math.h>
#include <stddef.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/mman.h>
#include <fcntl.h>
#include <unistd.h>

// minih264's SSE implementation defines __inline as an empty macro. Keep
// that internal choice inside its header; glibc fortify wrappers require the
// compiler's real inline semantics, as do headers used by the font renderer.
#pragma push_macro("__inline")
#define MINIH264_IMPLEMENTATION
#define H264E_MAX_THREADS 0
#define H264E_SVC_API 0
#include "minih264e.h"
#pragma pop_macro("__inline")
#define STB_TRUETYPE_IMPLEMENTATION
#include "stb_truetype.h"

#define WIDTH 800
#define HEIGHT 480
#define ROWS 7
#define LIMIT (512 * 1024)
typedef struct {
    unsigned char *font_bytes, *image;
    size_t font_size;
    stbtt_fontinfo font;
    H264E_persist_t *encoder;
    H264E_scratch_t *scratch;
} Menu;
static void *aligned(size_t n) { void *p = NULL; return posix_memalign(&p, 64, n) ? NULL : p; }
__attribute__((visibility("default"))) void carlink_menu_destroy(void *opaque) {
    Menu *m = opaque; if (!m) return;
    if (m->font_bytes) munmap(m->font_bytes, m->font_size);
    free(m->image); free(m->encoder); free(m->scratch); free(m);
}
__attribute__((visibility("default"))) void *carlink_menu_create(const char *path) {
    if (!path) return NULL;
    Menu *m = calloc(1, sizeof(*m)); if (!m) return NULL;
    int fd = open(path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC); struct stat st;
    if (fd < 0) goto failed;
    if (fstat(fd, &st) || !S_ISREG(st.st_mode) || st.st_size < 1024 || st.st_size > 24*1024*1024) { close(fd); goto failed; }
    // stbtt retains pointers into the font. Keep a read-only mapping for the
    // renderer lifetime so unused CJK glyph pages remain reclaimable/file-backed.
    size_t size = (size_t)st.st_size;
    void *font = mmap(NULL, size, PROT_READ, MAP_PRIVATE, fd, 0);
    close(fd);
    if (font == MAP_FAILED) goto failed;
    m->font_bytes = font; m->font_size = size;
    if (!stbtt_InitFont(&m->font, m->font_bytes, stbtt_GetFontOffsetForIndex(m->font_bytes, 0))) goto failed;
    m->image = aligned(WIDTH*HEIGHT*3/2); if (!m->image) goto failed;
    H264E_create_param_t parameters; memset(&parameters, 0, sizeof(parameters));
    parameters.width=WIDTH; parameters.height=HEIGHT; parameters.gop=1; parameters.const_input_flag=1;
#if defined(__ARM_NEON) || defined(__ARM_NEON__)
    parameters.enableNEON=1;
#endif
    int persist_size, scratch_size;
    if (H264E_sizeof(&parameters, &persist_size, &scratch_size)) goto failed;
    if (persist_size <= 0 || scratch_size <= 0 || persist_size > 16*1024*1024 || scratch_size > 16*1024*1024) goto failed;
    m->encoder=aligned((size_t)persist_size); m->scratch=aligned((size_t)scratch_size);
    if (!m->encoder || !m->scratch || H264E_init(m->encoder, &parameters)) goto failed;
    return m;
failed: carlink_menu_destroy(m); return NULL;
}
static void rectangle(Menu *m, int x, int y, int w, int h, unsigned char luma) {
    if (x<0 || y<0 || w<0 || h<0 || x+w>WIDTH || y+h>HEIGHT) return;
    for (int j=y; j<y+h; ++j) memset(m->image+j*WIDTH+x, luma, (size_t)w);
}
static void text(Menu *m, const uint32_t *points, size_t count, int x, int baseline, float height, unsigned char luma) {
    if (!points || count>128) return;
    float scale=stbtt_ScaleForPixelHeight(&m->font, height);
    for (size_t i=0; i<count && x<WIDTH-24; ++i) {
        uint32_t cp=points[i]; if (cp>0x10ffff || (cp>=0xd800 && cp<=0xdfff)) continue;
        int width, rows, xo, yo, advance, bearing;
        stbtt_GetCodepointHMetrics(&m->font, (int)cp, &advance, &bearing);
        unsigned char *bitmap=stbtt_GetCodepointBitmap(&m->font, 0, scale, (int)cp, &width, &rows, &xo, &yo);
        if (bitmap) {
            for (int by=0; by<rows; ++by) for (int bx=0; bx<width; ++bx) {
                int xx=x+xo+bx, yy=baseline+yo+by;
                if (xx>=24 && xx<WIDTH-24 && yy>=0 && yy<HEIGHT) {
                    unsigned char alpha=bitmap[by*width+bx], old=m->image[yy*WIDTH+xx];
                    m->image[yy*WIDTH+xx]=(unsigned char)(((unsigned)old*(255-alpha)+(unsigned)luma*alpha)/255);
                }
            }
            stbtt_FreeBitmap(bitmap, NULL);
        }
        x+=(int)(advance*scale+.5f);
    }
}
// Render up to 24 rows, focus scrolls over seven visible slots. Every output
// carries its own SPS/PPS/IDR: no reference to a different menu or phone frame.
__attribute__((visibility("default"))) int carlink_menu_render(void *opaque,
    const uint32_t *const *rows, const uint32_t *lengths, uint32_t count, uint32_t focus,
    unsigned char *output, uint32_t capacity) {
    Menu *m=opaque;
    if (!m || !rows || !lengths || !count || count>24 || focus>=count || !output || capacity<LIMIT) return -1;
    for (uint32_t i=0; i<count; ++i) if (!rows[i] || lengths[i]>128) return -1;
    memset(m->image, 25, WIDTH*HEIGHT); memset(m->image+WIDTH*HEIGHT, 128, WIDTH*HEIGHT/2);
    static const uint32_t title[]={'C','a','r','L','i','n','k',' ',0x83dc,0x5355};
    static const uint32_t footer[]={0x4e0a,0x4e0b,0xff1a,0x9009,0x62e9,' ',0x786e,0x8ba4,0xff1a,0x9009,0x62e9,' ',0x8fd4,0x56de,0xff1a,0x6295,0x5c4f};
    text(m,title,sizeof(title)/sizeof(*title),32,48,36,235);
    rectangle(m,24,62,752,2,90);
    uint32_t first=focus>=ROWS ? focus-ROWS+1 : 0;
    for (uint32_t slot=0; slot<ROWS && first+slot<count; ++slot) {
        uint32_t index=first+slot; int y=76+(int)slot*50;
        int selected=index==focus;
        if (selected) rectangle(m,24,y,752,46,210);
        text(m,rows[index],lengths[index],40,y+32,32,selected ? 25 : 225);
    }
    text(m,footer,sizeof(footer)/sizeof(*footer),32,458,24,170);
    H264E_io_yuv_t yuv={{m->image,m->image+WIDTH*HEIGHT,m->image+WIDTH*HEIGHT*5/4},{WIDTH,WIDTH/2,WIDTH/2}};
    H264E_run_param_t run; memset(&run,0,sizeof(run));
    run.encode_speed=H264E_SPEED_FASTEST; run.frame_type=H264E_FRAME_TYPE_KEY; run.qp_min=16; run.qp_max=16;
    unsigned char *coded=NULL; int bytes=0;
    if (H264E_encode(m->encoder,m->scratch,&run,&yuv,&coded,&bytes) || bytes<=0 || bytes>LIMIT || (uint32_t)bytes>capacity) return -1;
    memcpy(output,coded,(size_t)bytes); return bytes;
}
#ifdef CARLINK_MENU_TEST
int main(int argc,char **argv) {
    if (argc!=2) return 2;
    void *menu=carlink_menu_create(argv[1]); if (!menu) return 3;
    static const uint32_t row1[]={'C','a','r','P','l','a','y',' ',0x6211,0x7684,0x624b,0x673a};
    static const uint32_t row2[]={'H','i','C','a','r',' ',0x534e,0x4e3a,0x624b,0x673a};
    const uint32_t *rows[]={row1,row2}; uint32_t lengths[]={sizeof(row1)/4,sizeof(row2)/4};
    unsigned char *output=malloc(LIMIT); if (!output) return 4;
    if (carlink_menu_render(menu,rows,lengths,0,0,output,LIMIT)>=0 ||
        carlink_menu_render(menu,rows,lengths,2,2,output,LIMIT)>=0 ||
        carlink_menu_render(menu,rows,lengths,2,0,output,LIMIT-1)>=0) return 6;
    for (uint32_t focus=0; focus<2; ++focus) {
        int size=carlink_menu_render(menu,rows,lengths,2,focus,output,LIMIT);
        if (size<=0 || fwrite(output,1,(size_t)size,stdout)!=(size_t)size) return 5;
    }
    free(output); carlink_menu_destroy(menu); return 0;
}
#endif
