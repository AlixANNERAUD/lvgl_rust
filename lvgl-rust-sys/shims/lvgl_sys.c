#include "lvgl_sys.h"
#include "src/lvgl_private.h"

lv_color_t _LV_COLOR_MAKE(uint8_t r, uint8_t g, uint8_t b)
{
    return lv_color_make(r, g, b);
}

uint16_t _LV_COLOR_GET_R(lv_color_t color)
{
    return color.red;
}

uint16_t _LV_COLOR_GET_G(lv_color_t color)
{
    return color.green;
}

uint16_t _LV_COLOR_GET_B(lv_color_t color)
{
    return color.blue;
}

uint16_t _LV_COLOR_GET_A(lv_color_t color)
{
    /* lv_color_t is RGB888 in LVGL 9 and has no alpha channel. */
    (void)color;
    return 0;
}

bool lvgl_sys_obj_is_deleting(const lv_obj_t * obj)
{
    return obj->is_deleting;
}

bool lvgl_sys_obj_events_traversing(const lv_obj_t * obj)
{
    return obj->spec_attr != NULL && obj->spec_attr->event_list.is_traversing;
}

lv_point_t lvgl_sys_hit_test_point(const lv_hit_test_info_t * info)
{
    return *info->point;
}

bool lvgl_sys_hit_test_result(const lv_hit_test_info_t * info)
{
    return info->res;
}

void lvgl_sys_hit_test_set_result(lv_hit_test_info_t * info, bool res)
{
    info->res = res;
}

#if LV_USE_KEYBOARD
void lvgl_sys_keyboard_forget_textarea(lv_obj_t * keyboard)
{
    ((lv_keyboard_t *)keyboard)->ta = NULL;
}
#endif

#if LV_USE_OBJ_ID
void lvgl_sys_obj_set_id_raw(lv_obj_t * obj, void * id)
{
    obj->id = id;
}
#endif

bool lvgl_sys_display_reshape_buffers(lv_display_t * disp)
{
    if(disp->render_mode == LV_DISPLAY_RENDER_MODE_PARTIAL) return true;

    /* The buffer area of refr_area (lv_refr.c:908-925). */
    int32_t w = lv_display_get_horizontal_resolution(disp);
    int32_t h = lv_display_get_vertical_resolution(disp);
#if LV_DRAW_TRANSFORM_USE_MATRIX
    if(lv_display_get_matrix_rotation(disp)) {
        w = lv_display_get_original_horizontal_resolution(disp);
        h = lv_display_get_original_vertical_resolution(disp);
    }
#endif
    lv_draw_buf_t * buffers[3] = {disp->buf_1, disp->buf_2, disp->buf_3};
    bool fits = true;
    for(int i = 0; i < 3; i++) {
        lv_draw_buf_t * buffer = buffers[i];
        if(buffer == NULL) continue;
        uint32_t stride = disp->stride_is_auto ? LV_STRIDE_AUTO : buffer->header.stride;
        if(lv_draw_buf_reshape(buffer, disp->color_format, (uint32_t)w, (uint32_t)h, stride) == NULL) fits = false;
    }
    return fits;
}

void lvgl_sys_image_cache_drop(const void * src)
{
    lv_image_cache_drop(src);
}

#if LV_USE_MENU
lv_obj_t * lvgl_sys_menu_part(const lv_obj_t * menu, lvgl_sys_menu_part_t part)
{
    const lv_menu_t * m = (const lv_menu_t *)menu;
    switch(part) {
        case LVGL_SYS_MENU_PART_STORAGE:
            return m->storage;
        case LVGL_SYS_MENU_PART_MAIN:
            return m->main;
        case LVGL_SYS_MENU_PART_MAIN_HEADER:
            return m->main_header;
        case LVGL_SYS_MENU_PART_MAIN_HEADER_BACK_BUTTON:
            return m->main_header_back_btn;
        case LVGL_SYS_MENU_PART_MAIN_HEADER_TITLE:
            return m->main_header_title;
        default:
            break;
    }
    if(!m->sidebar_generated) return NULL;
    switch(part) {
        case LVGL_SYS_MENU_PART_SIDEBAR:
            return m->sidebar;
        case LVGL_SYS_MENU_PART_SIDEBAR_HEADER:
            return m->sidebar_header;
        case LVGL_SYS_MENU_PART_SIDEBAR_HEADER_BACK_BUTTON:
            return m->sidebar_header_back_btn;
        case LVGL_SYS_MENU_PART_SIDEBAR_HEADER_TITLE:
            return m->sidebar_header_title;
        default:
            return NULL;
    }
}
#endif
