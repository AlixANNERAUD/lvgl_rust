#include "lvgl.h"

#if LVGL_FONT_TEST_FLAG != 23
#error "external font archive did not receive exported custom LVGL defines"
#endif

static const uint8_t latin_glyph_bitmap[] = {0x3f, 0x00, 0x3f};
static const uint16_t latin_cmap[] = {0x20, 0x7e};
static const struct {
    const uint8_t *bitmap;
    const uint16_t *cmap;
} latin_data = {latin_glyph_bitmap, latin_cmap};

const lv_font_t font_montserrat_14_latin = {
    .line_height = 14,
    .base_line = 3,
    .static_bitmap = 1,
    .dsc = &latin_data,
};
