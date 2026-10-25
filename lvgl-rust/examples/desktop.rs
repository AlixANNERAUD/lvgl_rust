//! A small desktop in the spirit of Xila, on the host: a framebuffer
//! display, a scripted pointer, a top bar with a clock bound to a subject,
//! a launcher that opens windows (components with a close button, a
//! textarea and a keyboard), constant and mutable styles, a timer and an
//! animation. No heap feature: closures carry handles, exclusive work is
//! posted.
//!
//! Run with `mise run example` (it needs the configuration with OBJ_ID,
//! `lvgl-rust/conf`).

// The example keeps handles: views-only builds (`LVGL_RUST_HANDLE_SLOTS=0`)
// have none.
#[cfg(lvgl_rust_views_only)]
fn main() {
    eprintln!("the desktop example needs handles (LVGL_RUST_HANDLE_SLOTS > 0)");
}

#[cfg(not(lvgl_rust_views_only))]
fn main() {
    desktop::main();
}

#[cfg(not(lvgl_rust_views_only))]
mod desktop {
    use lvgl_rust::{
        Anim, AnimPath, Component, ConstStyle, Display, DisplayCell, DrawBuf, DrawBuffers, Error,
        Flush, FlushRequest, Handle, Indev, IndevCell, IndevData, IndevKind, IntSubject, Lvgl,
        LvglAccess, Obj, Read, RenderMode, Selector, Style, WidgetExt, color, const_style,
        enums::{Align, EventCode},
        tick_inc,
        widgets::{Button, Keyboard, Label, TextArea},
    };
    use std::sync::{
        Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering::Relaxed},
    };

    const WIDTH: usize = 320;
    const HEIGHT: usize = 240;

    /// The LVGL lock: every LVGL call happens with its guard.
    static LVGL: Mutex<()> = Mutex::new(());
    struct Guard(#[allow(dead_code)] MutexGuard<'static, ()>);
    // SAFETY: a guard of the mutex serializing every LVGL call of the program.
    unsafe impl LvglAccess for Guard {}

    // --- Drivers -------------------------------------------------------------

    /// Copies flushed areas into a framebuffer (on a device: SPI to a panel).
    struct Framebuffer {
        pixels: Vec<u8>,
        flushes: u32,
    }

    impl Flush for Framebuffer {
        fn flush(&mut self, mut req: FlushRequest<'_>) {
            let area = req.area();
            let stride = req.stride();
            let row_bytes = ((area.x2 - area.x1 + 1) as usize) * 4;
            let screen_stride = WIDTH * 4;
            for (y, row) in req.pixels().chunks(stride).enumerate() {
                let dst = (area.y1 as usize + y) * screen_stride + area.x1 as usize * 4;
                let n = row_bytes.min(row.len());
                if let Some(out) = self.pixels.get_mut(dst..dst + n) {
                    out.copy_from_slice(&row[..n]);
                }
            }
            self.flushes += 1;
        }
    }

    static PANEL: DisplayCell<Framebuffer> = DisplayCell::new();
    static BUF_A: DrawBuf<{ WIDTH * 4 * 24 }> = DrawBuf::new();
    static BUF_B: DrawBuf<{ WIDTH * 4 * 24 }> = DrawBuf::new();

    /// A pointer the script moves.
    struct Touch;
    static TOUCH_X: AtomicI32 = AtomicI32::new(0);
    static TOUCH_Y: AtomicI32 = AtomicI32::new(0);
    static TOUCH_DOWN: AtomicBool = AtomicBool::new(false);

    impl Read for Touch {
        fn read(&mut self, data: &mut IndevData<'_>) {
            data.set_point(TOUCH_X.load(Relaxed), TOUCH_Y.load(Relaxed));
            data.set_pressed(TOUCH_DOWN.load(Relaxed));
        }
    }

    static TOUCH: IndevCell<Touch> = IndevCell::new();

    // --- State and styles ----------------------------------------------------

    /// Seconds since start, shown by the clock (a label bound to it).
    static SECONDS: IntSubject = IntSubject::new(0);
    /// Open windows, shown in the bar.
    static OPEN: IntSubject = IntSubject::new(0);
    static OPENED: AtomicU32 = AtomicU32::new(0);
    static CLOSED: AtomicU32 = AtomicU32::new(0);

    static BAR: ConstStyle = const_style![
        bg_color(color::hex(0x1e1e2e)),
        radius(0),
        pad_top(4),
        pad_left(6)
    ];
    static WINDOW: ConstStyle = const_style![
        bg_color(color::hex(0xf5f5f5)),
        radius(8),
        border_width(1),
        pad_top(6)
    ];
    /// Changes with the theme (here: once, at start).
    static ACCENT: Style = Style::new();

    // --- A window component --------------------------------------------------

    struct Window {
        root: Handle<Obj>,
    }

    impl Window {
        /// Builds a window under `parent` (form (b): it needs exclusive calls
        /// for geometry and styles).
        fn build(parent: Handle<Obj>, lv: &mut Lvgl, number: u32) -> Result<Self, Error> {
            let root = Obj::create(parent.get(lv)?)?.handle()?;
            {
                let mut window = root.get_mut(lv)?;
                window.add_style(&WINDOW, Selector::DEFAULT);
                window.set_size(220, 150);
                window.align(Align::CENTER, (number as i32 % 3) * 10, 10);
            }
            let view = root.get(lv)?;
            let title = Label::create(view)?;
            title.set_text_i32(number as i32);
            // Views end before the exclusive calls below: keep handles.
            let title = title.handle()?;
            let close = Button::create(view)?;
            Label::create(close)?.set_text_str("x")?;
            // The close button deletes its window later: handlers cannot delete.
            close.on_with(EventCode::CLICKED, root, |e, root| {
                if let Ok(root) = root.get(e.lv())
                    && root.delete_later().is_ok()
                {
                    CLOSED.fetch_add(1, Relaxed);
                    let _ = e
                        .lv()
                        .post(|lv: &mut Lvgl, ()| OPEN.set(lv, OPEN.get(lv) - 1), ());
                }
            })?;
            let close = close.handle()?;
            let textarea = TextArea::create(view)?.handle()?;
            let keyboard = Keyboard::create(view)?.handle()?;
            {
                let mut close = close.upcast::<Obj>().get_mut(lv)?;
                close.align(Align::TOP_RIGHT, 0, 0);
                close.add_style(&ACCENT, Selector::DEFAULT);
            }
            {
                let mut ta = textarea.upcast::<Obj>().get_mut(lv)?;
                ta.set_size(200, 36);
                ta.align(Align::TOP_LEFT, 0, 24);
            }
            {
                let mut kb = keyboard.get_mut(lv)?;
                kb.set_textarea(Some(textarea))?;
                kb.as_base::<Obj>().set_size(200, 70);
                kb.as_base::<Obj>().align(Align::BOTTOM_MID, 0, 0);
            }
            // A short animation counting up in the title.
            title.upcast::<Obj>().get_mut(lv)?.animate(
                Anim::new(title)
                    .values(0, 100)
                    .duration(300)
                    .path(AnimPath::EaseOut)
                    .exec(|ctx, title, value| {
                        if let Ok(title) = title.get(ctx.lv()) {
                            title.set_text_i32(value);
                        }
                    }),
            )?;
            OPENED.fetch_add(1, Relaxed);
            OPEN.set(lv, OPEN.get(lv) + 1);
            Ok(Window { root })
        }
    }

    impl Component for Window {
        type Root = Obj;
        fn root(&self) -> Handle<Obj> {
            self.root
        }
    }

    /// Opens a window: posted by the launcher's handler, which has shared
    /// access only.
    fn open_window(lv: &mut Lvgl, number: u32) {
        let Some(screen) = lv.screen_active().and_then(|s| s.handle().ok()) else {
            return;
        };
        if let Err(error) = Window::build(screen, lv, number) {
            eprintln!("cannot open a window: {error:?}");
        }
    }

    // --- The program ---------------------------------------------------------

    fn frame(ms: u32) {
        let mut guard = Guard(LVGL.lock().unwrap());
        let mut lv = Lvgl::enter(&mut guard).unwrap();
        tick_inc(ms);
        lv.timer_handler();
    }

    fn click(x: i32, y: i32) {
        TOUCH_X.store(x, Relaxed);
        TOUCH_Y.store(y, Relaxed);
        TOUCH_DOWN.store(true, Relaxed);
        frame(40);
        TOUCH_DOWN.store(false, Relaxed);
        frame(40);
    }

    pub fn main() {
        {
            let mut guard = Guard(LVGL.lock().unwrap());
            let mut lv = lvgl_rust::init(&mut guard).unwrap();
            let framebuffer = Framebuffer {
                pixels: vec![0; WIDTH * HEIGHT * 4],
                flushes: 0,
            };
            // Dropping a `Display` or an `Indev` keeps the device.
            let _display = Display::create(
                &mut lv,
                &PANEL,
                framebuffer,
                WIDTH as u32,
                HEIGHT as u32,
                DrawBuffers::double(&BUF_A, &BUF_B),
                RenderMode::Partial,
            )
            .unwrap();
            Indev::create(&mut lv, &TOUCH, Touch, IndevKind::Pointer).unwrap();
            lv.style(&ACCENT).set_bg_color(color::hex(0x89b4fa));

            let screen = lv.screen_active().unwrap().handle().unwrap();
            // The top bar: a clock and the number of open windows.
            let bar = Obj::create(screen.get(&lv).unwrap())
                .unwrap()
                .handle()
                .unwrap();
            {
                let mut bar = bar.get_mut(&mut lv).unwrap();
                bar.add_style(&BAR, Selector::DEFAULT);
                bar.set_size(WIDTH as i32, 28);
            }
            let clock = Label::create(bar.get(&lv).unwrap())
                .unwrap()
                .handle()
                .unwrap();
            clock.get_mut(&mut lv).unwrap().bind_text(&SECONDS).unwrap();
            let open = Label::create(bar.get(&lv).unwrap())
                .unwrap()
                .handle()
                .unwrap();
            {
                let mut open = open.get_mut(&mut lv).unwrap();
                open.bind_text(&OPEN).unwrap();
                open.as_base::<Obj>().align(Align::RIGHT_MID, 0, 0);
            }
            // A timer ticks the clock (setting a subject is exclusive: posted).
            // Forgotten: dropping an `Owned<Timer>` deletes the timer.
            std::mem::forget(
                lv.timer(1000, (), |ctx, ()| {
                    let _ = ctx
                        .lv()
                        .post(|lv: &mut Lvgl, ()| SECONDS.set(lv, SECONDS.get(lv) + 1), ());
                })
                .unwrap(),
            );
            // The launcher.
            let launcher = Button::create(screen.get(&lv).unwrap()).unwrap();
            Label::create(launcher)
                .unwrap()
                .set_text_str("Open")
                .unwrap();
            launcher
                .on(EventCode::CLICKED, |e| {
                    let number = OPENED.load(Relaxed) + 1;
                    let _ = e.lv().post(open_window, number);
                })
                .unwrap();
            let launcher = launcher.handle().unwrap();
            let mut launcher = launcher.upcast::<Obj>().get_mut(&mut lv).unwrap();
            launcher.set_size(80, 40);
            launcher.align(Align::BOTTOM_LEFT, 8, -8);
        }

        // The script: two windows, close one, let the clock run.
        frame(40);
        click(40, HEIGHT as i32 - 28); // open
        click(40, HEIGHT as i32 - 28); // open a second one
        for _ in 0..10 {
            frame(40);
        }
        // The second window's close button: top right of a centred 220 × 150
        // window shifted by (20, 10).
        click(
            WIDTH as i32 / 2 + 20 + 110 - 16,
            HEIGHT as i32 / 2 + 10 - 75 + 14,
        );
        for _ in 0..60 {
            frame(50);
        }

        let mut guard = Guard(LVGL.lock().unwrap());
        let lv = Lvgl::enter(&mut guard).unwrap();
        let seconds = SECONDS.get(&lv);
        let open = OPEN.get(&lv);
        println!(
            "opened {} windows, closed {}, {} open; the clock reads {} s",
            OPENED.load(Relaxed),
            CLOSED.load(Relaxed),
            open,
            seconds
        );
        assert_eq!(OPENED.load(Relaxed), 2, "the launcher opened two windows");
        assert_eq!(CLOSED.load(Relaxed), 1, "one close button was clicked");
        assert_eq!(open, 1);
        assert!(seconds >= 3, "the clock ticked");
    }
}
