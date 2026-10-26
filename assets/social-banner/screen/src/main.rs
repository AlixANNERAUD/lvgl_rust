//! Renders the smart-home panel shown on the social banner through the
//! safe API, into the PNG file given as the first argument.

use lvgl_rust::{
    Display, DisplayCell, DrawBuf, DrawBuffers, Flush, FlushRequest, Font, Lvgl, LvglAccess, Obj,
    RenderMode, Selector, WidgetExt, color,
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

    let screen = lv.screen_active().unwrap().handle().unwrap();
    screen
        .get_mut(&mut lv)
        .unwrap()
        .set_style_bg_color(color::hex(0x101418), Selector::DEFAULT);

    // Title row.
    let title = Label::create(screen.get(&lv).unwrap()).unwrap();
    title.set_text_str("Living room").unwrap();
    let title = title.handle().unwrap();
    {
        let mut title = title.get_mut(&mut lv).unwrap();
        let mut base = title.as_base::<Obj>();
        base.set_style_text_font(Font::montserrat_20(), Selector::DEFAULT);
        base.align(Align::TOP_LEFT, 20, 18);
    }
    let clock = Label::create(screen.get(&lv).unwrap()).unwrap();
    clock.set_text_str("21:42").unwrap();
    let clock = clock.handle().unwrap();
    {
        let mut clock = clock.get_mut(&mut lv).unwrap();
        let mut base = clock.as_base::<Obj>();
        base.set_style_text_font(Font::montserrat_16(), Selector::DEFAULT);
        base.set_style_text_color(color::hex(0x8b949e), Selector::DEFAULT);
        base.align(Align::TOP_RIGHT, -20, 20);
    }

    // Thermostat arc.
    let arc = Arc::create(screen.get(&lv).unwrap()).unwrap();
    arc.set_range(10, 30);
    arc.set_value(21);
    let arc = arc.handle().unwrap();
    {
        let mut arc = arc.get_mut(&mut lv).unwrap();
        let mut base = arc.as_base::<Obj>();
        base.set_size(190, 190);
        base.align(Align::LEFT_MID, 22, 22);
    }
    let temperature = Label::create(arc.get(&lv).unwrap()).unwrap();
    temperature.set_text_str("21.5°").unwrap();
    let temperature = temperature.handle().unwrap();
    {
        let mut t = temperature.get_mut(&mut lv).unwrap();
        let mut base = t.as_base::<Obj>();
        base.set_style_text_font(Font::montserrat_40(), Selector::DEFAULT);
        base.align(Align::CENTER, 0, -6);
    }
    let caption = Label::create(arc.get(&lv).unwrap()).unwrap();
    caption.set_text_str("Heating").unwrap();
    let caption = caption.handle().unwrap();
    {
        let mut c = caption.get_mut(&mut lv).unwrap();
        let mut base = c.as_base::<Obj>();
        base.set_style_text_color(color::hex(0x8b949e), Selector::DEFAULT);
        base.align(Align::CENTER, 0, 30);
    }

    // Controls column.
    let lights = Label::create(screen.get(&lv).unwrap()).unwrap();
    lights.set_text_str("Lights").unwrap();
    let lights = lights.handle().unwrap();
    {
        let mut l = lights.get_mut(&mut lv).unwrap();
        let mut base = l.as_base::<Obj>();
        base.set_style_text_font(Font::montserrat_16(), Selector::DEFAULT);
        base.align(Align::TOP_LEFT, 250, 78);
    }
    let switch = Switch::create(screen.get(&lv).unwrap())
        .unwrap()
        .handle()
        .unwrap();
    {
        let mut s = switch.get_mut(&mut lv).unwrap();
        let mut base = s.as_base::<Obj>();
        base.add_state(State::CHECKED);
        base.align(Align::TOP_RIGHT, -22, 72);
    }
    let brightness = Label::create(screen.get(&lv).unwrap()).unwrap();
    brightness.set_text_str("Brightness  70%").unwrap();
    let brightness = brightness.handle().unwrap();
    {
        let mut l = brightness.get_mut(&mut lv).unwrap();
        let mut base = l.as_base::<Obj>();
        base.set_style_text_font(Font::montserrat_16(), Selector::DEFAULT);
        base.align(Align::TOP_LEFT, 250, 130);
    }
    let slider = Slider::create(screen.get(&lv).unwrap()).unwrap();
    slider
        .on(EventCode::VALUE_CHANGED, |e| {
            let _ = e.lv().post(set_brightness, 0);
        })
        .unwrap();
    let slider = slider.handle().unwrap();
    {
        let mut s = slider.get_mut(&mut lv).unwrap();
        s.view().set_value(70, false);
        let mut base = s.as_base::<Obj>();
        base.set_width(206);
        base.align(Align::TOP_LEFT, 252, 166);
    }
    let scenes = Button::create(screen.get(&lv).unwrap())
        .unwrap()
        .handle()
        .unwrap();
    {
        let mut b = scenes.get_mut(&mut lv).unwrap();
        let mut base = b.as_base::<Obj>();
        base.set_size(206, 44);
        base.align(Align::TOP_LEFT, 250, 214);
    }
    let scenes_label = Label::create(scenes.get(&lv).unwrap()).unwrap();
    scenes_label.set_text_str("Evening scene").unwrap();
    let scenes_label = scenes_label.handle().unwrap();
    {
        let mut l = scenes_label.get_mut(&mut lv).unwrap();
        l.as_base::<Obj>().align(Align::CENTER, 0, 0);
    }

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
fn banner_snippet(screen: &Obj) -> Result<(), lvgl_rust::Error> {
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
