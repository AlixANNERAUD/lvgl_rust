//! Renders the smart-home panel shown on the social banner through the
//! safe API, into the PNG file given as the first argument.

use lvgl_rust::{
    Display, DisplayCell, DrawBuf, DrawBuffers, Flush, FlushRequest, Font, Lvgl, LvglAccess, Obj,
    ObjMut, RenderMode, Selector, color,
    enums::{Align, EventCode, State},
    tick_inc,
    widgets::{Arc, Button, Label, Slider, Switch},
};
use std::sync::{Mutex, MutexGuard};

const WIDTH: usize = 480;
const HEIGHT: usize = 300;

static LVGL: Mutex<()> = Mutex::new(());
struct Guard(#[allow(dead_code)] MutexGuard<'static, ()>);
// SAFETY: a guard of the mutex serializing every LVGL call of the program.
unsafe impl LvglAccess for Guard {}

static PIXELS: Mutex<Vec<u8>> = Mutex::new(Vec::new());

struct Framebuffer;

impl Flush for Framebuffer {
    fn flush(&mut self, mut req: FlushRequest<'_>) {
        let area = req.area();
        let stride = req.stride();
        let row_bytes = ((area.x2 - area.x1 + 1) as usize) * 4;
        for (y, row) in req.pixels().chunks(stride).enumerate() {
            let dst = (area.y1 as usize + y) * WIDTH * 4 + area.x1 as usize * 4;
            let n = row_bytes.min(row.len());
            if let Some(out) = PIXELS.lock().unwrap().get_mut(dst..dst + n) {
                out.copy_from_slice(&row[..n]);
            }
        }
    }
}

static PANEL: DisplayCell<Framebuffer> = DisplayCell::new();
static BUF: DrawBuf<{ WIDTH * HEIGHT * 4 }> = DrawBuf::new();

fn set_brightness(_lv: &mut Lvgl, _value: i32) {}

fn main() {
    let mut guard = Guard(LVGL.lock().unwrap());
    *PIXELS.lock().unwrap() = vec![0; WIDTH * HEIGHT * 4];
    let mut lv = lvgl_rust::init(&mut guard).unwrap();
    let _display = Display::create(
        &mut lv,
        &PANEL,
        Framebuffer,
        WIDTH as u32,
        HEIGHT as u32,
        DrawBuffers::single(&BUF),
        RenderMode::Full,
    )
    .unwrap();

    let mut screen = lv.screen_active_mut().unwrap();
    screen.set_style_bg_color(color::hex(0x101418), Selector::DEFAULT);

    // Title row.
    let mut title = Label::create(&mut screen).unwrap();
    title.view().set_text_str("Living room").unwrap();
    title.set_style_text_font(Font::montserrat_20(), Selector::DEFAULT);
    title.align(Align::TOP_LEFT, 20, 18);
    let mut clock = Label::create(&mut screen).unwrap();
    clock.view().set_text_str("21:42").unwrap();
    clock.set_style_text_font(Font::montserrat_16(), Selector::DEFAULT);
    clock.set_style_text_color(color::hex(0x8b949e), Selector::DEFAULT);
    clock.align(Align::TOP_RIGHT, -20, 20);

    // Thermostat arc.
    let mut arc = Arc::create(&mut screen).unwrap();
    arc.view().set_range(10, 30);
    arc.view().set_value(21);
    arc.set_size(190, 190);
    arc.align(Align::LEFT_MID, 22, 22);
    let mut temperature = Label::create(&mut arc).unwrap();
    temperature.view().set_text_str("21.5°").unwrap();
    temperature.set_style_text_font(Font::montserrat_40(), Selector::DEFAULT);
    temperature.align(Align::CENTER, 0, -6);
    let mut caption = Label::create(&mut arc).unwrap();
    caption.view().set_text_str("Heating").unwrap();
    caption.set_style_text_color(color::hex(0x8b949e), Selector::DEFAULT);
    caption.align(Align::CENTER, 0, 30);

    // Controls column.
    let mut lights = Label::create(&mut screen).unwrap();
    lights.view().set_text_str("Lights").unwrap();
    lights.set_style_text_font(Font::montserrat_16(), Selector::DEFAULT);
    lights.align(Align::TOP_LEFT, 250, 78);
    let mut switch = Switch::create(&mut screen).unwrap();
    switch.add_state(State::CHECKED);
    switch.align(Align::TOP_RIGHT, -22, 72);
    let mut brightness = Label::create(&mut screen).unwrap();
    brightness.view().set_text_str("Brightness  70%").unwrap();
    brightness.set_style_text_font(Font::montserrat_16(), Selector::DEFAULT);
    brightness.align(Align::TOP_LEFT, 250, 130);
    let mut slider = Slider::create(&mut screen).unwrap();
    slider
        .view()
        .on(EventCode::VALUE_CHANGED, |e| {
            let _ = e.lv().post(set_brightness, 0);
        })
        .unwrap();
    slider.view().set_value(70, false);
    slider.set_width(206);
    slider.align(Align::TOP_LEFT, 252, 166);
    let mut scenes = Button::create(&mut screen).unwrap();
    scenes.set_size(206, 44);
    scenes.align(Align::TOP_LEFT, 250, 214);
    let mut scenes_label = Label::create(&mut scenes).unwrap();
    scenes_label.view().set_text_str("Evening scene").unwrap();
    scenes_label.align(Align::CENTER, 0, 0);

    for _ in 0..5 {
        tick_inc(100);
        lv.timer_handler();
    }

    let pixels = PIXELS.lock().unwrap().clone();
    let mut rgba = Vec::with_capacity(WIDTH * HEIGHT * 4);
    for px in pixels.chunks(4) {
        rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
    }
    let output = std::env::args().nth(1).expect("output PNG path");
    let file = std::fs::File::create(output).unwrap();
    let mut encoder = png::Encoder::new(file, WIDTH as u32, HEIGHT as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&rgba)
        .unwrap();
}

// The snippet printed on the banner, compiled so it stays true.
#[allow(dead_code)]
fn toggle_lights(_lv: &mut Lvgl, _: ()) {}

#[allow(dead_code)]
fn banner_snippet(screen: &mut ObjMut<'_, Obj>) -> Result<(), lvgl_rust::Error> {
    let arc = Arc::create(screen)?;
    arc.view().set_range(10, 30);
    arc.view().set_value(21);

    let lights = Switch::create(screen)?;
    lights.view().on(EventCode::VALUE_CHANGED, |e| {
        // Handlers get shared access:
        // exclusive work is posted.
        let _ = e.lv().post(toggle_lights, ());
    })?;
    Ok(())
}
