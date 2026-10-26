//! Widgets whose internals LVGL trusts: chart series, table cells, tile
//! views and calendar headers.

use lvgl_rust::{
    ChartSeries, Error, Lvgl, LvglAccess, Obj, Session, TABLE_CELL_LIMIT, color,
    enums::{ChartAxis, ChartType, TableCellCtrl},
    sys,
    widgets::{Chart, Label, Roller, Table, TileView},
};
use std::sync::{Mutex, MutexGuard};

static LOCK: Mutex<bool> = Mutex::new(false);

struct Guard(MutexGuard<'static, bool>);

// SAFETY: a guard of the mutex serializing every test's LVGL calls.
unsafe impl LvglAccess for Guard {}

fn guard() -> Guard {
    Guard(LOCK.lock().unwrap_or_else(|p| p.into_inner()))
}

fn session(guard: &mut Guard) -> Session<'_> {
    if !*guard.0 {
        *guard.0 = true;
        let lv = lvgl_rust::init(guard).unwrap();
        // SAFETY: session held; the buffer is static and only LVGL uses it.
        unsafe {
            let display = sys::lv_display_create(240, 160);
            sys::lv_display_set_flush_cb(display, Some(flush));
            sys::lv_display_set_buffers(
                display,
                core::ptr::addr_of_mut!(BUFFER).cast(),
                core::ptr::null_mut(),
                core::mem::size_of::<Buffer>() as u32,
                sys::lv_display_render_mode_t_LV_DISPLAY_RENDER_MODE_PARTIAL,
            );
        }
        lv
    } else {
        Lvgl::enter(guard).unwrap()
    }
}

/// A partial draw buffer: 20 lines of 32-bit pixels.
#[repr(C, align(64))]
struct Buffer([u8; 240 * 20 * 4]);
static mut BUFFER: Buffer = Buffer([0; 240 * 20 * 4]);

unsafe extern "C" fn flush(display: *mut sys::lv_display_t, _: *const sys::lv_area_t, _: *mut u8) {
    // SAFETY: called by LVGL for this display.
    unsafe { sys::lv_display_flush_ready(display) };
}

/// Lays out and draws the active screen.
fn draw() {
    // SAFETY: called with a session held.
    unsafe { sys::lv_refr_now(core::ptr::null_mut()) };
}

fn last(lv: &Lvgl) -> Option<&Obj> {
    lv.screen_active()?.child(-1)
}

fn last_as<W: lvgl_rust::Widget>(lv: &Lvgl) -> Option<&W> {
    last(lv)?.downcast::<W>()
}

#[test]
fn chart_series_belong_to_their_chart() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let screen = lv.screen_active().unwrap();
    let first = Chart::create(screen).unwrap();
    let second = Chart::create(screen).unwrap();
    second.set_point_count(4).unwrap();
    first.set_point_count(4).unwrap();
    let series = first
        .add_series(color::hex(0xff0000), ChartAxis::PRIMARY_Y)
        .unwrap();
    let other = second
        .add_series(color::hex(0x00ff00), ChartAxis::PRIMARY_Y)
        .unwrap();
    first.set_series_values(&series, &[1, 2, 3]).unwrap();
    first.set_next_value(&series, 4).unwrap();
    first.set_series_value_by_id(&series, 9, 99).unwrap(); // ignored by LVGL
    let mut values = [0; 8];
    assert_eq!(first.get_series_y_values_into(&series, &mut values), Ok(4));
    assert_eq!(&values[..4], &[1, 2, 3, 4]);
    let mut short = [0; 2];
    assert_eq!(first.get_series_y_values_into(&series, &mut short), Ok(2));
    assert_eq!(first.get_series_x_values_into(&series, &mut values), Ok(0));
    first
        .set_series_color(&series, color::hex(0x0000ff))
        .unwrap();
    assert_eq!(
        first.get_series_color(&series).unwrap().blue,
        color::hex(0x0000ff).blue
    );
    first.hide_series(&series, false).unwrap();
    first.set_x_start_point(&series, 1).unwrap();
    assert_eq!(first.get_x_start_point(&series), Ok(1));
    first.get_point_pos_by_id(&series, 2).unwrap();
    assert_eq!(
        first.set_series_values2(&series, &[1], &[1, 2]),
        Err(Error::InvalidArg)
    );
    // A series of another chart is refused, whatever the call.
    assert_eq!(second.set_next_value(&series, 1), Err(Error::InvalidArg));
    assert_eq!(second.get_x_start_point(&series), Err(Error::InvalidArg));
    assert_eq!(first.set_all_values(&other, 0), Err(Error::InvalidArg));
    assert_eq!(second.remove_series(series), Err(Error::InvalidArg));
    draw();

    // Scatter charts take pairs.
    let scatter = first
        .add_series(color::hex(0), ChartAxis::PRIMARY_Y)
        .unwrap();
    first.set_type(ChartType::SCATTER);
    first.set_next_value2(&scatter, 5, 6).unwrap();
    first
        .set_series_values2(&scatter, &[7, 8], &[9, 10])
        .unwrap();
    first.set_series_value_by_id2(&scatter, 0, 1, 2).unwrap();
    assert_eq!(first.get_series_x_values_into(&scatter, &mut values), Ok(4));
    draw();
    first.remove_series(scatter).unwrap();

    // The first chart's series die with it: a token of a deleted chart is
    // refused by the chart that is left.
    let leftover = first
        .add_series(color::hex(0), ChartAxis::SECONDARY_Y)
        .unwrap();
    lv.edit(|lv| lv.screen_active()?.child(-2))
        .unwrap()
        .delete()
        .unwrap();
    let second = last_as::<Chart>(&lv).unwrap();
    assert_eq!(second.set_next_value(&leftover, 1), Err(Error::InvalidArg));
    assert_eq!(second.remove_series(leftover), Err(Error::InvalidArg));
    second.remove_series(other).unwrap();
    // SAFETY: session held, live chart.
    assert!(unsafe { sys::lv_chart_get_series_next(second.as_ptr(), core::ptr::null()) }.is_null());
    lv.edit(last).unwrap().delete().unwrap();
}

#[test]
fn chart_series_are_tokens() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ChartSeries>();
    assert_eq!(
        core::mem::size_of::<ChartSeries>(),
        core::mem::size_of::<usize>()
    );
    assert_eq!(
        core::mem::size_of::<Option<ChartSeries>>(),
        core::mem::size_of::<usize>()
    );
}

#[test]
fn table_cells_are_bounds_checked() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let table = Table::create(lv.screen_active().unwrap()).unwrap();
    let mut buf = [0u8; 16];
    // A new table has one empty cell.
    assert_eq!(table.get_cell_value_into(0, 0, &mut buf), Ok(c""));
    assert_eq!(
        table.get_cell_value_into(1, 0, &mut buf),
        Err(Error::InvalidArg)
    );
    assert_eq!(
        table.get_cell_value_into(0, u32::MAX, &mut buf),
        Err(Error::InvalidArg)
    );
    assert_eq!(table.get_cell_user_data(0, 1), Err(Error::InvalidArg));
    assert_eq!(table.get_cell_user_data(0, 0), Ok(core::ptr::null_mut()));
    // Setters grow the table.
    table.set_cell_value(2, 1, c"cell").unwrap();
    assert_eq!(table.get_row_count(), 3);
    assert_eq!(table.get_column_count(), 2);
    assert_eq!(table.get_cell_value_into(2, 1, &mut buf), Ok(c"cell"));
    assert_eq!(
        table.get_cell_value_into(2, 1, &mut [0u8; 4]),
        Err(Error::TooSmall)
    );
    table
        .set_cell_ctrl(1, 1, TableCellCtrl::TEXT_CROP | TableCellCtrl::CUSTOM_1)
        .unwrap();
    assert!(table.has_cell_ctrl(1, 1, TableCellCtrl::TEXT_CROP));
    table
        .clear_cell_ctrl(1, 1, TableCellCtrl::TEXT_CROP)
        .unwrap();
    assert!(!table.has_cell_ctrl(1, 1, TableCellCtrl::TEXT_CROP));
    assert!(!table.has_cell_ctrl(9, 9, TableCellCtrl::CUSTOM_1));
    table.set_column_width(3, 40).unwrap();
    assert_eq!(table.get_column_count(), 4);
    // Indexes whose `+ 1` wraps, and sizes LVGL would compute in 32 bits.
    assert_eq!(
        table.set_cell_value(u32::MAX, 0, c"x"),
        Err(Error::InvalidArg)
    );
    assert_eq!(
        table.set_cell_ctrl(0, u32::MAX, TableCellCtrl::CUSTOM_1),
        Err(Error::InvalidArg)
    );
    assert_eq!(table.set_column_width(u32::MAX, 1), Err(Error::InvalidArg));
    assert_eq!(table.set_row_count(u32::MAX), Err(Error::InvalidArg));
    assert_eq!(
        table.set_row_count(TABLE_CELL_LIMIT / 4 + 1),
        Err(Error::InvalidArg)
    );
    assert_eq!(table.set_column_count(1 << 30), Err(Error::InvalidArg));
    assert_eq!(table.get_row_count(), 3);
    draw();
    // Shrinking frees cells; their getters are refused afterwards.
    table.set_row_count(1).unwrap();
    assert_eq!(
        table.get_cell_value_into(2, 1, &mut buf),
        Err(Error::InvalidArg)
    );
    table.set_cell_value(0, 0, c"top").unwrap();
    let mut edit = lv.edit(last_as::<Table>).unwrap();
    assert_eq!(edit.get_cell_value(0, 0), Ok(c"top"));
    assert_eq!(edit.get_cell_value(0, 4), Err(Error::InvalidArg));
    edit.set_selected_cell(0, 9);
    assert_eq!(edit.view().get_selected_cell(), (0, 3));
    edit.delete().unwrap();
}

#[test]
fn closed_widgets_take_no_foreign_children() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let screen = lv.screen_active().unwrap();
    let roller = Roller::create(screen).unwrap();
    // The roller casts its first child to its label.
    assert_eq!(Obj::create(roller).err(), Some(Error::Internal));
    assert_eq!(Label::create(roller).err(), Some(Error::Internal));
    let tileview = TileView::create(screen).unwrap();
    assert_eq!(Obj::create(tileview).err(), Some(Error::Internal));
    assert_eq!(Label::create(tileview).err(), Some(Error::Internal));
    lv.edit(last).unwrap().delete().unwrap();
    lv.edit(last).unwrap().delete().unwrap();
}

#[cfg(not(lvgl_rust_views_only))]
mod handles {
    use super::*;
    use lvgl_rust::{WidgetExt, enums::Dir, widgets::Calendar};

    #[test]
    fn tile_views_hold_only_tiles() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let screen = lv.screen_active().unwrap();
        let tileview = TileView::create(screen).unwrap().handle().unwrap();
        let stranger = Obj::create(screen).unwrap().handle().unwrap();
        let (first, second) = {
            let mut edit = tileview.get_mut(&mut lv).unwrap();
            let first = edit.add_tile(0, 0, Dir::RIGHT).unwrap().handle().unwrap();
            let second = edit.add_tile(1, 0, Dir::LEFT).unwrap().handle().unwrap();
            (first, second)
        };
        // Tiles take the application's children.
        let content = Label::create(first.get(&lv).unwrap())
            .unwrap()
            .handle()
            .unwrap();
        // Nothing else moves in, and only tiles can be shown.
        assert_eq!(
            stranger
                .get_mut(&mut lv)
                .unwrap()
                .set_parent(tileview.upcast()),
            Err(Error::Internal)
        );
        let mut edit = tileview.get_mut(&mut lv).unwrap();
        assert_eq!(edit.set_tile(stranger, false), Err(Error::InvalidArg));
        assert_eq!(
            edit.set_tile(content.upcast(), false),
            Err(Error::InvalidArg)
        );
        edit.set_tile(second, false).unwrap();
        let active = tileview.get(&lv).unwrap().get_tile_active().unwrap();
        assert_eq!(active.as_ptr(), second.get(&lv).unwrap().as_ptr());
        draw();
        // The active tile can go: LVGL keeps the stale pointer, the wrapper
        // no longer returns it.
        second.get_mut(&mut lv).unwrap().delete().unwrap();
        assert!(tileview.get(&lv).unwrap().get_tile_active().is_none());
        draw();
        let mut edit = tileview.get_mut(&mut lv).unwrap();
        edit.set_tile_by_index(1, 0, false); // no tile there any more
        edit.set_tile_by_index(0, 0, false);
        let active = tileview.get(&lv).unwrap().get_tile_active().unwrap();
        assert_eq!(active.as_ptr(), first.get(&lv).unwrap().as_ptr());
        // A tile moved out is no longer the active one.
        let mut edit = first.get_mut(&mut lv).unwrap();
        edit.set_parent(stranger).unwrap();
        assert!(tileview.get(&lv).unwrap().get_tile_active().is_none());
        // ... and cannot move back.
        assert_eq!(
            first
                .get_mut(&mut lv)
                .unwrap()
                .set_parent(tileview.upcast()),
            Err(Error::Internal)
        );
        content.get_mut(&mut lv).unwrap().delete().unwrap();
        tileview.get_mut(&mut lv).unwrap().delete().unwrap();
        stranger.get_mut(&mut lv).unwrap().delete().unwrap();
    }

    #[test]
    fn zero_size_tile_views_ignore_scroll_end() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let screen = lv.screen_active().unwrap();
        let tileview = TileView::create(screen).unwrap().handle().unwrap();
        let mut edit = tileview.get_mut(&mut lv).unwrap();
        edit.add_tile(0, 0, Dir::ALL).unwrap();
        // LVGL's handler would divide by the content width or height: the
        // guard stops the SCROLL_END sent by an unanimated scroll.
        edit.set_width(0);
        edit.update_layout();
        edit.scroll_by(0, 10, false);
        edit.set_width(100);
        edit.set_height(0);
        edit.update_layout();
        edit.scroll_by(10, 0, false);
        // The handler never ran: it would have made the tile at (0, 0)
        // active.
        assert!(edit.view().get_tile_active().is_none());
        edit.set_height(100);
        edit.update_layout();
        edit.scroll_by(10, 0, false);
        assert!(edit.view().get_tile_active().is_some());
        draw();
        tileview.get_mut(&mut lv).unwrap().delete().unwrap();
    }

    #[test]
    fn calendar_headers_stay_in_their_calendar() {
        let mut guard = guard();
        let mut lv = session(&mut guard);
        let screen = lv.screen_active().unwrap();
        let screen_handle = screen.handle().unwrap();
        let calendar = Calendar::create(screen).unwrap().handle().unwrap();
        let header = {
            let mut edit = calendar.get_mut(&mut lv).unwrap();
            assert_eq!(edit.set_month_shown(2024, 13), Err(Error::InvalidArg));
            assert_eq!(edit.set_month_shown(2024, 0), Err(Error::InvalidArg));
            assert_eq!(edit.set_shown_month(13), Err(Error::InvalidArg));
            edit.set_month_shown(2025, 12).unwrap();
            edit.add_header_arrow().unwrap().handle().unwrap()
        };
        calendar
            .get_mut(&mut lv)
            .unwrap()
            .set_shown_month(2)
            .unwrap();
        calendar.get_mut(&mut lv).unwrap().set_shown_year(2026);
        draw();
        // The button matrix and the header's buttons and label are parts.
        let view = calendar.get(&lv).unwrap();
        let matrix = view.get_btnmatrix().unwrap();
        assert_eq!(matrix.delete_later(), Err(Error::Internal));
        assert_eq!(view.clean_later(), Err(Error::Internal));
        let header_view = header.get(&lv).unwrap();
        for i in 0..3 {
            let part = header_view.child(i).unwrap();
            assert_eq!(part.delete_later(), Err(Error::Internal));
            assert_eq!(part.own().err(), Some(Error::Internal));
        }
        assert_eq!(header_view.clean_later(), Err(Error::Internal));
        assert_eq!(Obj::create(header_view).err(), Some(Error::Internal));
        let label = header_view.child(1).unwrap().handle().unwrap();
        assert_eq!(
            label.get_mut(&mut lv).unwrap().move_to_index(0),
            Err(Error::Internal)
        );
        // The header reads its parent as the calendar: it never moves.
        assert_eq!(
            header.get_mut(&mut lv).unwrap().set_parent(screen_handle),
            Err(Error::Internal)
        );
        // The calendar's other children are the application's.
        let user = Obj::create(calendar.get(&lv).unwrap())
            .unwrap()
            .handle()
            .unwrap();
        calendar
            .get_mut(&mut lv)
            .unwrap()
            .set_shown_month(3)
            .unwrap();
        user.get_mut(&mut lv).unwrap().delete().unwrap();
        // The header itself can go.
        header.get_mut(&mut lv).unwrap().delete().unwrap();
        calendar
            .get_mut(&mut lv)
            .unwrap()
            .set_shown_month(4)
            .unwrap();
        calendar.get_mut(&mut lv).unwrap().delete().unwrap();
    }
}
