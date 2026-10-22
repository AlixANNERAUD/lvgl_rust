#include "lvgl.h"

#if LVGL_FONT_TEST_FLAG != 23
#error "external font archive did not receive exported custom LVGL defines"
#endif

static const uint8_t cjk_glyph_bitmap[] = {0x7f, 0x01, 0x7f};
static const uint16_t cjk_cmap[] = {0x4e00, 0x9fff};
static const struct {
    const uint8_t *bitmap;
    const uint16_t *cmap;
} cjk_data = {cjk_glyph_bitmap, cjk_cmap};

const lv_font_t font_montserrat_14_cjk = {
    .line_height = 14,
    .base_line = 3,
    .static_bitmap = 1,
    .dsc = &cjk_data,
};
