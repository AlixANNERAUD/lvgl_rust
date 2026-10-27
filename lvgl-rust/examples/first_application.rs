//! The program of the tutorial "Your first LVGL application in Rust"
//! (`documentation/tutorials/first-application.md`): a display that renders
//! into memory, a scripted pointer, and a button whose clicks are counted
//! in a label. It checks what LVGL rendered, so it needs no screen.
//!
//! Run with `mise run example` (it needs the configuration with OBJ_ID,
//! `lvgl-rust/conf`).

// The program keeps handles: views-only builds (`LVGL_RUST_HANDLE_SLOTS=0`)
// have none.
#[cfg(lvgl_rust_views_only)]
fn main() {
    eprintln!("the first application needs handles (LVGL_RUST_HANDLE_SLOTS > 0)");
}

#[cfg(not(lvgl_rust_views_only))]
fn main() -> Result<(), lvgl_rust::Error> {
    first_application::main()
}

#[cfg(not(lvgl_rust_views_only))]
mod first_application {
    use lvgl_rust::{
        Display, DisplayCell, DrawBuf, DrawBuffers, Error, Flush, FlushRequest, Handle, Indev,
        IndevCell, IndevData, IndevKind, Lvgl, LvglAccess, Read, RenderMode, WidgetExt,
        enums::{Align, EventCode},
        tick_inc,
        widgets::{Button, Label},
    };
    use std::sync::{
        Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering::Relaxed},
    };

    /// The LVGL lock: every LVGL call happens while its guard is alive.
    static LVGL: Mutex<()> = Mutex::new(());

    /// Proof that the program holds the LVGL lock.
    struct Guard(#[allow(dead_code)] MutexGuard<'static, ()>);

    // SAFETY: a guard of the mutex that serializes every LVGL call of the
    // program.
    unsafe impl LvglAccess for Guard {}

    const WIDTH: usize = 320;
    const HEIGHT: usize = 240;
    /// `lv_conf.h` renders in RGB565: 2 bytes per pixel.
    const BYTES_PER_PIXEL: usize = 2;

    /// The pixels of the screen, as a panel would hold them.
    static FRAMEBUFFER: Mutex<[u8; WIDTH * HEIGHT * BYTES_PER_PIXEL]> =
        Mutex::new([0; WIDTH * HEIGHT * BYTES_PER_PIXEL]);
    /// How many areas LVGL has sent to the display.
    static FLUSHES: AtomicU32 = AtomicU32::new(0);

    /// The display driver: copies each area LVGL renders into `FRAMEBUFFER`.
    struct Panel;

    impl Flush for Panel {
        fn flush(&mut self, mut request: FlushRequest<'_>) {
            let area = request.area();
            let stride = request.stride();
            let row_bytes = (area.x2 - area.x1 + 1) as usize * BYTES_PER_PIXEL;
            let mut framebuffer = FRAMEBUFFER.lock().unwrap();
            for (y, row) in request.pixels().chunks(stride).enumerate() {
                let start = ((area.y1 as usize + y) * WIDTH + area.x1 as usize) * BYTES_PER_PIXEL;
                let n = row_bytes.min(row.len());
                if let Some(out) = framebuffer.get_mut(start..start + n) {
                    out.copy_from_slice(&row[..n]);
                }
            }
            FLUSHES.fetch_add(1, Relaxed);
        }
    }

    /// Where the display driver lives while the display exists.
    static PANEL: DisplayCell<Panel> = DisplayCell::new();
    /// LVGL renders into this buffer: 24 rows of the screen at a time.
    static DRAW_BUFFER: DrawBuf<{ WIDTH * BYTES_PER_PIXEL * 24 }> = DrawBuf::new();

    /// The position and state of a pretend mouse, moved by `click`.
    static POINTER_X: AtomicI32 = AtomicI32::new(0);
    static POINTER_Y: AtomicI32 = AtomicI32::new(0);
    static POINTER_DOWN: AtomicBool = AtomicBool::new(false);

    /// The input driver: reports the pretend mouse to LVGL.
    struct Pointer;

    impl Read for Pointer {
        fn read(&mut self, data: &mut IndevData<'_>) {
            data.set_point(POINTER_X.load(Relaxed), POINTER_Y.load(Relaxed));
            data.set_pressed(POINTER_DOWN.load(Relaxed));
        }
    }

    /// Where the input driver lives while the input device exists.
    static POINTER: IndevCell<Pointer> = IndevCell::new();

    /// The number of clicks so far.
    static CLICKS: AtomicU32 = AtomicU32::new(0);

    /// Counts a click and shows the new count in `label`. The button's
    /// handler posts it: it runs once the handler has returned, with
    /// exclusive access to LVGL.
    fn count_click(lv: &mut Lvgl, label: Handle<Label>) {
        let clicks = CLICKS.fetch_add(1, Relaxed) + 1;
        if let Ok(label) = label.get(lv) {
            label.set_text_u32(clicks);
        }
    }

    /// Builds the screen: a label showing the count, and a button below
    /// it. Returns the label's handle.
    fn build(lv: &mut Lvgl) -> Result<Handle<Label>, Error> {
        let (count, button) = {
            let mut screen = lv.screen_active_mut().expect("the display has a screen");
            let count = Label::create(&mut screen)?;
            count.set_text_u32(0);
            let count = count.handle()?;
            let button = Button::create(&mut screen)?.handle()?;
            (count, button)
        };
        count.get_mut(lv)?.align(Align::TOP_MID, 0, 40);

        let mut button = button.get_mut(lv)?;
        button.set_size(120, 50);
        button.center();
        Label::create(&mut button)?.set_text_str("Click me")?;
        button
            .view()
            .on_with(EventCode::CLICKED, count, |event, count| {
                let _ = event.lv().post(count_click, count);
            })?;
        Ok(count)
    }

    /// Lets `ms` milliseconds pass: LVGL reads the pointer, runs the
    /// handlers and the posted functions, and redraws what changed.
    fn frame(ms: u32) {
        let mut guard = Guard(LVGL.lock().unwrap());
        let mut lv = Lvgl::enter(&mut guard).unwrap();
        tick_inc(ms);
        lv.timer_handler();
    }

    /// Presses and releases the pretend mouse at (`x`, `y`).
    fn click(x: i32, y: i32) {
        POINTER_X.store(x, Relaxed);
        POINTER_Y.store(y, Relaxed);
        POINTER_DOWN.store(true, Relaxed);
        frame(40);
        POINTER_DOWN.store(false, Relaxed);
        frame(40);
    }

    /// The RGB565 pixel at (`x`, `y`) of the framebuffer.
    fn pixel(x: usize, y: usize) -> u16 {
        let framebuffer = FRAMEBUFFER.lock().unwrap();
        let start = (y * WIDTH + x) * BYTES_PER_PIXEL;
        u16::from_le_bytes([framebuffer[start], framebuffer[start + 1]])
    }

    pub fn main() -> Result<(), Error> {
        let count = {
            let mut guard = Guard(LVGL.lock().unwrap());
            let mut lv = lvgl_rust::init(&mut guard)?;
            // Dropping a `Display` or an `Indev` keeps the device.
            let _display = Display::create(
                &mut lv,
                &PANEL,
                Panel,
                WIDTH as u32,
                HEIGHT as u32,
                DrawBuffers::single(&DRAW_BUFFER),
                RenderMode::Partial,
            )?;
            let _pointer = Indev::create(&mut lv, &POINTER, Pointer, IndevKind::Pointer)?;
            build(&mut lv)?
        };

        // Draw the screen once, click the button three times, and let LVGL
        // draw the new count.
        frame(40);
        for _ in 0..3 {
            click(WIDTH as i32 / 2, HEIGHT as i32 / 2);
        }
        for _ in 0..5 {
            frame(40);
        }

        let mut guard = Guard(LVGL.lock().unwrap());
        let lv = Lvgl::enter(&mut guard)?;
        let mut text = [0u8; 12];
        let shown = count.get(&lv)?.get_text_into(&mut text)?;
        // Inside the button, left of its text, and the screen's corner.
        let button_pixel = pixel(WIDTH / 2 - 50, HEIGHT / 2);
        let background_pixel = pixel(4, HEIGHT - 4);
        println!(
            "{} clicks, the label shows {:?}, {} flushes, button pixel {:#06x}, background pixel {:#06x}",
            CLICKS.load(Relaxed),
            shown,
            FLUSHES.load(Relaxed),
            button_pixel,
            background_pixel
        );
        assert_eq!(CLICKS.load(Relaxed), 3, "three clicks");
        assert_eq!(shown, Some(c"3"), "the label shows the count");
        assert!(FLUSHES.load(Relaxed) > 0, "LVGL sent pixels to the display");
        assert_ne!(button_pixel, background_pixel, "the button is drawn");
        Ok(())
    }
}
