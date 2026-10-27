//! Renders the smart-home panel shown on the social banner through the
//! safe API, into the PNG file given as the first argument.

use lvgl_rust::{
    Display, DisplayCell, DrawBuf, DrawBuffers, Flush, FlushRequest, Font, Handle, IsA, Lvgl,
    LvglAccess, Obj, ObjMut, RenderMode, Selector, Widget, WidgetExt, color,
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

/// Exclusive setup of an object kept by handle.
fn edit<W: Widget + IsA<Obj>>(
    lv: &mut Lvgl,
    object: Handle<W>,
    f: impl FnOnce(&mut ObjMut<'_, Obj>),
) {
    f(&mut object.upcast::<Obj>().get_mut(lv).unwrap());
}

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

    // Created through the screen's `ObjMut`, each lent as a view for its
    // shared setup, then kept by handle for its exclusive setup below.
    let title = Label::create(&mut screen).unwrap();
    title.set_text_str("Living room").unwrap();
    let title = title.handle().unwrap();
    let clock = Label::create(&mut screen).unwrap();
    clock.set_text_str("21:42").unwrap();
    let clock = clock.handle().unwrap();
    let arc = Arc::create(&mut screen).unwrap();
    arc.set_range(10, 30);
    arc.set_value(21);
    let arc = arc.handle().unwrap();
    let lights = Label::create(&mut screen).unwrap();
    lights.set_text_str("Lights").unwrap();
    let lights = lights.handle().unwrap();
    let switch = Switch::create(&mut screen).unwrap().handle().unwrap();
    let brightness = Label::create(&mut screen).unwrap();
    brightness.set_text_str("Brightness  70%").unwrap();
    let brightness = brightness.handle().unwrap();
    let slider = Slider::create(&mut screen).unwrap();
    slider
        .on(EventCode::VALUE_CHANGED, |e| {
            let _ = e.lv().post(set_brightness, 0);
        })
        .unwrap();
    slider.set_value(70, false);
    let slider = slider.handle().unwrap();
    let scenes = Button::create(&mut screen).unwrap().handle().unwrap();

    // Title row.
    edit(&mut lv, title, |o| {
        o.set_style_text_font(Font::montserrat_20(), Selector::DEFAULT);
        o.align(Align::TOP_LEFT, 20, 18);
    });
    edit(&mut lv, clock, |o| {
        o.set_style_text_font(Font::montserrat_16(), Selector::DEFAULT);
        o.set_style_text_color(color::hex(0x8b949e), Selector::DEFAULT);
        o.align(Align::TOP_RIGHT, -20, 20);
    });

    // Thermostat arc.
    edit(&mut lv, arc, |o| {
        o.set_size(190, 190);
        o.align(Align::LEFT_MID, 22, 22);
        let temperature = Label::create(o).unwrap();
        temperature.set_text_str("21.5°").unwrap();
        let caption = Label::create(o).unwrap();
        caption.set_text_str("Heating").unwrap();
    });
    let temperature = arc.get(&lv).unwrap().child(0).unwrap().handle().unwrap();
    edit(&mut lv, temperature, |o| {
        o.set_style_text_font(Font::montserrat_40(), Selector::DEFAULT);
        o.align(Align::CENTER, 0, -6);
    });
    let caption = arc.get(&lv).unwrap().child(1).unwrap().handle().unwrap();
    edit(&mut lv, caption, |o| {
        o.set_style_text_color(color::hex(0x8b949e), Selector::DEFAULT);
        o.align(Align::CENTER, 0, 30);
    });

    // Controls column.
    edit(&mut lv, lights, |o| {
        o.set_style_text_font(Font::montserrat_16(), Selector::DEFAULT);
        o.align(Align::TOP_LEFT, 250, 78);
    });
    edit(&mut lv, switch, |o| {
        o.add_state(State::CHECKED);
        o.align(Align::TOP_RIGHT, -22, 72);
    });
    edit(&mut lv, brightness, |o| {
        o.set_style_text_font(Font::montserrat_16(), Selector::DEFAULT);
        o.align(Align::TOP_LEFT, 250, 130);
    });
    edit(&mut lv, slider, |o| {
        o.set_width(206);
        o.align(Align::TOP_LEFT, 252, 166);
    });
    edit(&mut lv, scenes, |o| {
        o.set_size(206, 44);
        o.align(Align::TOP_LEFT, 250, 214);
        Label::create(o)
            .unwrap()
            .set_text_str("Evening scene")
            .unwrap();
    });
    let scenes_label = scenes.get(&lv).unwrap().child(0).unwrap().handle().unwrap();
    edit(&mut lv, scenes_label, |o| o.align(Align::CENTER, 0, 0));

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
    arc.set_range(10, 30);
    arc.set_value(21);

    let lights = Switch::create(screen)?;
    lights.on(EventCode::VALUE_CHANGED, |e| {
        // Handlers get shared access:
        // exclusive work is posted.
        let _ = e.lv().post(toggle_lights, ());
    })?;
    Ok(())
}
