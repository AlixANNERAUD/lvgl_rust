#ifndef LVGL_API_H
#define LVGL_API_H

#ifdef __cplusplus
extern "C"
{
#endif

#include "../lvgl/lvgl.h"

    lv_color_t _LV_COLOR_MAKE(uint8_t r, uint8_t g, uint8_t b);
    uint16_t _LV_COLOR_GET_R(lv_color_t color);
    uint16_t _LV_COLOR_GET_G(lv_color_t color);
    uint16_t _LV_COLOR_GET_B(lv_color_t color);
    uint16_t _LV_COLOR_GET_A(lv_color_t color);

    /** Whether `obj` is being deleted (its `LV_EVENT_DELETE` was sent). */
    bool lvgl_sys_obj_is_deleting(const lv_obj_t * obj);

    /**
     * Whether LVGL is sending an event to `obj` right now (its event list is
     * being traversed): adding a handler may reallocate the list under a
     * copy of its header that LVGL frees if the object is deleted during
     * the send (lv_event.c:111, 145).
     */
    bool lvgl_sys_obj_events_traversing(const lv_obj_t * obj);

#if LV_USE_KEYBOARD
    /**
     * Makes a keyboard forget its textarea without touching it (unlike
     * `lv_keyboard_set_textarea(kb, NULL)`), for when the textarea is being
     * deleted.
     */
    void lvgl_sys_keyboard_forget_textarea(lv_obj_t * keyboard);
#endif

#if LV_USE_OBJ_ID
    /** Stores `id` in the object without calling `lv_obj_free_id` or
     *  `lv_obj_assign_id`, for applications that implement the OBJ_ID hooks. */
    void lvgl_sys_obj_set_id_raw(lv_obj_t * obj, void * id);
#endif

    /**
     * Gives every draw buffer of `disp` the shape the next refresh renders
     * into (the whole screen in full and direct modes, `lv_refr.c:908-925`),
     * after a change of resolution, rotation, color format or render mode.
     * LVGL reshapes only the active buffer, when it renders into it; in
     * direct mode with two buffers it first copies areas between them with
     * their previous shapes (`lv_refr.c:780`). Nothing to do in partial
     * mode, where each area is reshaped before rendering. Returns false if
     * a buffer is too small for the new shape (it is then left unchanged).
     */
    bool lvgl_sys_display_reshape_buffers(lv_display_t * disp);

    /** Removes the image cache entries decoded from `src` (forwards to
     *  `lv_image_cache_drop`, which LVGL declares in a private header). */
    void lvgl_sys_image_cache_drop(const void * src);

#if LV_USE_MENU
    /** Parts of a menu that LVGL caches in the widget. */
    typedef enum {
        LVGL_SYS_MENU_PART_STORAGE,
        LVGL_SYS_MENU_PART_MAIN,
        LVGL_SYS_MENU_PART_MAIN_HEADER,
        LVGL_SYS_MENU_PART_MAIN_HEADER_BACK_BUTTON,
        LVGL_SYS_MENU_PART_MAIN_HEADER_TITLE,
        LVGL_SYS_MENU_PART_SIDEBAR,
        LVGL_SYS_MENU_PART_SIDEBAR_HEADER,
        LVGL_SYS_MENU_PART_SIDEBAR_HEADER_BACK_BUTTON,
        LVGL_SYS_MENU_PART_SIDEBAR_HEADER_TITLE,
    } lvgl_sys_menu_part_t;

    /** A cached part of `menu`, without the deprecation log of the public
     *  getters. Sidebar parts are NULL unless a sidebar is shown:
     *  `lv_menu_set_sidebar_page(menu, NULL)` deletes the sidebar but leaves
     *  the cached pointers dangling. NULL for an unknown part. */
    lv_obj_t * lvgl_sys_menu_part(const lv_obj_t * menu, lvgl_sys_menu_part_t part);
#endif

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /*LVGL_API*/
