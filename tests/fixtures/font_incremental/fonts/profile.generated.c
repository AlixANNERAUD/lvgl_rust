#include "lvgl.h"

static const uint8_t glyph_bitmap[] = {0x01, 0x02, 0x03};

const lv_font_t font_incremental_profile = {
    .line_height = 12,
    .base_line = 2,
    .static_bitmap = 1,
    .dsc = glyph_bitmap,
};
