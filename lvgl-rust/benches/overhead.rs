//! What lvgl-rust adds to raw `lvgl_rust::sys` calls doing the same LVGL
//! work: each benchmark times the safe path and its raw equivalent on the
//! host, in nanoseconds per operation (median and minimum of 21 samples,
//! after a warm-up batch; the two paths' samples alternate).
//!
//! Run with `mise run overhead`, which writes
//! `documentation/reference/overhead.md`. With `LVGL_RUST_OVERHEAD_OUTPUT`
//! set to a directory, the tables are also written there as Markdown
//! fragments (`performance.md`, `types.md`). Raw calls include the crate's
//! OBJ_ID hooks: LVGL calls them for every object, safe or not.

// The benchmark keeps handles: views-only builds (`LVGL_RUST_HANDLE_SLOTS=0`)
// have none.
#[cfg(lvgl_rust_views_only)]
fn main() {
    eprintln!("the overhead benchmark needs handles (LVGL_RUST_HANDLE_SLOTS > 0)");
}

#[cfg(not(lvgl_rust_views_only))]
fn main() {
    overhead::main();
}

#[cfg(not(lvgl_rust_views_only))]
mod overhead {
    use lvgl_rust::{
        Display, DisplayCell, DrawBuf, DrawBuffers, Flush, FlushRequest, Handle, Lvgl, LvglAccess,
        Obj, ObjMut, Owned, RenderMode, Session, WidgetExt,
        enums::{EventCode, SafeEventCode},
        sys,
        widgets::Label,
    };
    use std::{
        ffi::c_void,
        fmt::Write as _,
        hint::black_box,
        mem::size_of,
        ptr::{self, NonNull},
        sync::atomic::{AtomicU32, Ordering::Relaxed},
        time::Instant,
    };

    const SAMPLES: usize = 21;
    /// Iterations per sample: calls that only touch an object, and calls
    /// that create or delete one (or start a session).
    const CHEAP: u32 = 100_000;
    const HEAVY: u32 = 5_000;

    /// The access token. `main` makes the only value, and the benchmark is
    /// single-threaded.
    struct Token;
    // SAFETY: one value, on one thread, borrowed for each session in turn.
    unsafe impl LvglAccess for Token {}

    /// A display that is never flushed: no tick ever makes the refresh
    /// timer due.
    struct NoPanel;
    impl Flush for NoPanel {
        fn flush(&mut self, _: FlushRequest<'_>) {}
    }

    static PANEL: DisplayCell<NoPanel> = DisplayCell::new();
    static BUF: DrawBuf<{ 64 * 4 * 8 }> = DrawBuf::new();

    /// What every handler and posted function does.
    static HITS: AtomicU32 = AtomicU32::new(0);

    fn hit() {
        HITS.store(HITS.load(Relaxed).wrapping_add(1), Relaxed);
    }

    unsafe extern "C" fn raw_handler(_: *mut sys::lv_event_t) {
        hit();
    }

    unsafe extern "C" fn raw_handler_data(e: *mut sys::lv_event_t) {
        // SAFETY: a live event during its dispatch.
        black_box(unsafe { sys::lv_event_get_user_data(e) });
        hit();
    }

    #[inline(never)]
    fn bump(_: &mut Lvgl, data: u32) {
        black_box(data);
        hit();
    }

    const READY: sys::lv_event_code_t = sys::lv_event_code_t_LV_EVENT_READY;

    /// Objects shared by the benchmarks.
    struct Fixture {
        /// A plain object on the active screen: the parent of created
        /// objects, and the target of setters.
        panel: Handle<Obj>,
        label: Handle<Label>,
        /// One object per handler kind, each with one handler.
        on: *mut sys::lv_obj_t,
        on_with: *mut sys::lv_obj_t,
        #[cfg(feature = "heap")]
        on_boxed: *mut sys::lv_obj_t,
        raw: *mut sys::lv_obj_t,
        raw_data: *mut sys::lv_obj_t,
    }

    impl Fixture {
        fn new(lv: &mut Lvgl) -> Self {
            let panel = Obj::create(&mut lv.screen_active_mut().expect("screen"))
                .expect("panel")
                .handle()
                .expect("handle");
            let label = Label::create(&mut lv.screen_active_mut().expect("screen"))
                .expect("label")
                .handle()
                .expect("handle");
            let on = {
                let mut screen = lv.screen_active_mut().expect("screen");
                let o = Obj::create(&mut screen).expect("object");
                o.on(EventCode::READY, |_| hit()).expect("on");
                o.as_ptr()
            };
            let on_with = {
                let mut screen = lv.screen_active_mut().expect("screen");
                let o = Obj::create(&mut screen).expect("object");
                o.on_with(EventCode::READY, 7u32, |_, data| {
                    black_box(data);
                    hit();
                })
                .expect("on_with");
                o.as_ptr()
            };
            #[cfg(feature = "heap")]
            let on_boxed = {
                let mut screen = lv.screen_active_mut().expect("screen");
                let o = Obj::create(&mut screen).expect("object");
                let data = 7u32;
                o.on_boxed(EventCode::READY, move |_| {
                    black_box(data);
                    hit();
                })
                .expect("on_boxed");
                o.as_ptr()
            };
            let screen = lv.screen_active().expect("screen").as_ptr();
            // SAFETY: exclusive access; live screen; the callbacks need no
            // data beyond the pointer value.
            let (raw, raw_data) = unsafe {
                let raw = sys::lv_obj_create(screen);
                sys::lv_obj_add_event_cb(raw, Some(raw_handler), READY, ptr::null_mut());
                let raw_data = sys::lv_obj_create(screen);
                sys::lv_obj_add_event_cb(
                    raw_data,
                    Some(raw_handler_data),
                    READY,
                    ptr::without_provenance_mut::<c_void>(7),
                );
                (raw, raw_data)
            };
            Self {
                panel,
                label,
                on,
                on_with,
                #[cfg(feature = "heap")]
                on_boxed,
                raw,
                raw_data,
            }
        }

        fn panel_ptr(&self, lv: &Lvgl) -> *mut sys::lv_obj_t {
            self.panel.get(lv).expect("panel").as_ptr()
        }
    }

    #[derive(Clone, Copy)]
    struct Stats {
        median: f64,
        min: f64,
    }

    /// The mean time per iteration of `batch(token, iterations)`.
    fn sample(token: &mut Token, iterations: u32, batch: &mut impl FnMut(&mut Token, u32)) -> f64 {
        let start = Instant::now();
        batch(token, iterations);
        start.elapsed().as_nanos() as f64 / f64::from(iterations)
    }

    fn stats(mut samples: [f64; SAMPLES]) -> Stats {
        samples.sort_by(f64::total_cmp);
        Stats {
            median: samples[SAMPLES / 2],
            min: samples[0],
        }
    }

    /// Times the safe path alone (its raw equivalent does nothing): a
    /// warm-up batch, then `SAMPLES`.
    fn alone(
        token: &mut Token,
        iterations: u32,
        mut safe: impl FnMut(&mut Token, u32),
    ) -> (Stats, Option<Stats>) {
        safe(token, iterations);
        let samples = core::array::from_fn(|_| sample(token, iterations, &mut safe));
        (stats(samples), None)
    }

    /// Times the safe path and its raw equivalent, alternating samples so
    /// that both see the same machine load.
    fn pair(
        token: &mut Token,
        iterations: u32,
        mut safe: impl FnMut(&mut Token, u32),
        mut raw: impl FnMut(&mut Token, u32),
    ) -> (Stats, Option<Stats>) {
        safe(token, iterations);
        raw(token, iterations);
        let mut safe_samples = [0f64; SAMPLES];
        let mut raw_samples = [0f64; SAMPLES];
        for (s, r) in safe_samples.iter_mut().zip(&mut raw_samples) {
            *s = sample(token, iterations, &mut safe);
            *r = sample(token, iterations, &mut raw);
        }
        (stats(safe_samples), Some(stats(raw_samples)))
    }

    struct Row {
        operation: &'static str,
        safe: &'static str,
        raw: &'static str,
        safe_stats: Stats,
        /// `None`: the raw equivalent does nothing.
        raw_stats: Option<Stats>,
    }

    fn enter(token: &mut Token) -> Session<'_> {
        Lvgl::enter(token).expect("enter")
    }

    fn rows(token: &mut Token, fx: &Fixture) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut row = |operation, safe, raw, (safe_stats, raw_stats): (Stats, Option<Stats>)| {
            rows.push(Row {
                operation,
                safe,
                raw,
                safe_stats,
                raw_stats,
            })
        };

        // Sessions.
        row(
            "Start and end a session (nothing pending)",
            "`Lvgl::enter` + drop",
            "nothing",
            alone(token, CHEAP, |t, n| {
                for _ in 0..n {
                    drop(black_box(enter(t)));
                }
            }),
        );

        // Shared calls through views and handles.
        row(
            "Shared setter through a view",
            "`obj.set_ext_click_area(v)`",
            "`lv_obj_set_ext_click_area(p, v)`",
            pair(
                token,
                CHEAP,
                |t, n| {
                    let lv = enter(t);
                    let obj = fx.panel.get(&lv).expect("panel");
                    for i in 0..n {
                        black_box(obj).set_ext_click_area(black_box(i as i32 & 7));
                    }
                },
                |t, n| {
                    let lv = enter(t);
                    let p = fx.panel_ptr(&lv);
                    for i in 0..n {
                        // SAFETY: session held, live object.
                        unsafe {
                            sys::lv_obj_set_ext_click_area(black_box(p), black_box(i as i32 & 7))
                        };
                    }
                },
            ),
        );
        row(
            "Resolve a handle, then a shared setter",
            "`handle.get(&lv)?.set_ext_click_area(v)`",
            "`lv_obj_set_ext_click_area(p, v)`",
            pair(
                token,
                CHEAP,
                |t, n| {
                    let lv = enter(t);
                    for i in 0..n {
                        black_box(fx.panel)
                            .get(&lv)
                            .expect("panel")
                            .set_ext_click_area(black_box(i as i32 & 7));
                    }
                },
                |t, n| {
                    let lv = enter(t);
                    let p = fx.panel_ptr(&lv);
                    for i in 0..n {
                        // SAFETY: session held, live object.
                        unsafe {
                            sys::lv_obj_set_ext_click_area(black_box(p), black_box(i as i32 & 7))
                        };
                    }
                },
            ),
        );
        row(
            "Resolve a `Handle<Obj>` (no class check)",
            "`Handle<Obj>::get`",
            "nothing (keep the pointer)",
            alone(token, CHEAP, |t, n| {
                let lv = enter(t);
                for _ in 0..n {
                    black_box(black_box(fx.panel).get(&lv).expect("panel"));
                }
            }),
        );
        row(
            "Resolve a `Handle<Label>` (class check)",
            "`Handle<Label>::get`",
            "nothing (keep the pointer)",
            alone(token, CHEAP, |t, n| {
                let lv = enter(t);
                for _ in 0..n {
                    black_box(black_box(fx.label).get(&lv).expect("label"));
                }
            }),
        );
        row(
            "Exclusive access through a handle",
            "`Handle<Obj>::get_mut`",
            "nothing (keep the pointer)",
            alone(token, CHEAP, |t, n| {
                let mut lv = enter(t);
                for _ in 0..n {
                    let edit: ObjMut<'_, Obj> =
                        black_box(fx.panel).get_mut(&mut lv).expect("panel");
                    black_box(edit);
                }
            }),
        );

        // Creating and deleting objects.
        let raw_create_delete = |t: &mut Token, n: u32| {
            let lv = enter(t);
            let parent = fx.panel_ptr(&lv);
            for _ in 0..n {
                // SAFETY: session held, live parent; the child is deleted
                // at once.
                unsafe {
                    let child = sys::lv_obj_create(black_box(parent));
                    sys::lv_obj_delete(black_box(child));
                }
            }
        };
        row(
            "Create and delete an object",
            "`Obj::create(&mut parent)`, then `lv.edit(..)` + `ObjMut::delete`",
            "`lv_obj_create` + `lv_obj_delete`",
            pair(
                token,
                HEAVY,
                |t, n| {
                    let mut lv = enter(t);
                    for _ in 0..n {
                        Obj::create(&mut fx.panel.get_mut(&mut lv).expect("panel"))
                            .expect("create");
                        lv.edit(|lv| fx.panel.get(lv).ok()?.child(-1))
                            .expect("child")
                            .delete()
                            .expect("delete");
                    }
                },
                raw_create_delete,
            ),
        );
        row(
            "Create an object, take a handle, delete it",
            "`Obj::create(..)?.handle()`, then `get_mut` + `delete`",
            "`lv_obj_create` + `lv_obj_delete`",
            pair(
                token,
                HEAVY,
                |t, n| {
                    let mut lv = enter(t);
                    for _ in 0..n {
                        let child = Obj::create(&mut fx.panel.get_mut(&mut lv).expect("panel"))
                            .expect("create")
                            .handle()
                            .expect("handle");
                        child
                            .get_mut(&mut lv)
                            .expect("child")
                            .delete()
                            .expect("delete");
                    }
                },
                raw_create_delete,
            ),
        );
        row(
            "Owned object dropped, deleted by the drain",
            "`Obj::create_owned` + drop, drained by the next `Lvgl::enter`",
            "`lv_obj_create` + `lv_obj_delete`",
            pair(
                token,
                HEAVY,
                |t, n| {
                    for _ in 0..n {
                        let mut lv = enter(t);
                        let owned: Owned<Obj> =
                            Obj::create_owned(&mut fx.panel.get_mut(&mut lv).expect("panel"))
                                .expect("create");
                        drop(owned);
                    }
                    drop(enter(t));
                },
                raw_create_delete,
            ),
        );
        row(
            "Deferred deletion",
            "`lv_obj_create`, `delete_later` + drain by the next `Lvgl::enter`",
            "`lv_obj_create` + `lv_obj_delete`",
            pair(
                token,
                HEAVY,
                |t, n| {
                    for _ in 0..n {
                        let lv = enter(t);
                        let parent = fx.panel_ptr(&lv);
                        // SAFETY: session held, live parent.
                        let child =
                            NonNull::new(unsafe { sys::lv_obj_create(parent) }).expect("create");
                        // SAFETY: just created, live.
                        unsafe { Obj::from_raw(&lv, child) }
                            .delete_later()
                            .expect("delete_later");
                    }
                    drop(enter(t));
                },
                raw_create_delete,
            ),
        );

        // Events.
        let raw_send = |t: &mut Token, n: u32| {
            let _lv = enter(t);
            for _ in 0..n {
                // SAFETY: session held, live object; READY reads no parameter.
                unsafe { sys::lv_obj_send_event(black_box(fx.raw), READY, ptr::null_mut()) };
            }
        };
        row(
            "Event to a handler without captures",
            "`obj.on(code, |e| ..)`, sent with `lv_obj_send_event`",
            "`extern \"C\"` callback from `lv_obj_add_event_cb`",
            pair(
                token,
                CHEAP,
                |t, n| {
                    let _lv = enter(t);
                    for _ in 0..n {
                        // SAFETY: as above.
                        unsafe { sys::lv_obj_send_event(black_box(fx.on), READY, ptr::null_mut()) };
                    }
                },
                raw_send,
            ),
        );
        row(
            "Event to a handler with 4 bytes of data",
            "`obj.on_with(code, data, |e, data| ..)`, sent with `lv_obj_send_event`",
            "`extern \"C\"` callback reading its user data",
            pair(
                token,
                CHEAP,
                |t, n| {
                    let _lv = enter(t);
                    for _ in 0..n {
                        // SAFETY: as above.
                        unsafe {
                            sys::lv_obj_send_event(black_box(fx.on_with), READY, ptr::null_mut())
                        };
                    }
                },
                |t, n| {
                    let _lv = enter(t);
                    for _ in 0..n {
                        // SAFETY: as above.
                        unsafe {
                            sys::lv_obj_send_event(black_box(fx.raw_data), READY, ptr::null_mut())
                        };
                    }
                },
            ),
        );
        #[cfg(feature = "heap")]
        row(
            "Event to a boxed closure (feature `heap`)",
            "`obj.on_boxed(code, move |e| ..)`, sent with `lv_obj_send_event`",
            "`extern \"C\"` callback reading its user data",
            pair(
                token,
                CHEAP,
                |t, n| {
                    let _lv = enter(t);
                    for _ in 0..n {
                        // SAFETY: as above.
                        unsafe {
                            sys::lv_obj_send_event(black_box(fx.on_boxed), READY, ptr::null_mut())
                        };
                    }
                },
                |t, n| {
                    let _lv = enter(t);
                    for _ in 0..n {
                        // SAFETY: as above.
                        unsafe {
                            sys::lv_obj_send_event(black_box(fx.raw_data), READY, ptr::null_mut())
                        };
                    }
                },
            ),
        );
        row(
            "Safe send to a handler without captures",
            "`obj.send_event(SafeEventCode::READY)` to an `on` handler",
            "`lv_obj_send_event` to an `extern \"C\"` callback",
            pair(
                token,
                CHEAP,
                |t, n| {
                    let lv = enter(t);
                    // SAFETY: live object, not being deleted.
                    let obj = unsafe { Obj::from_raw(&lv, NonNull::new(fx.on).expect("object")) };
                    for _ in 0..n {
                        black_box(obj)
                            .send_event(SafeEventCode::READY)
                            .expect("send");
                    }
                },
                raw_send,
            ),
        );

        // Deferred work and the timer handler.
        row(
            "Posted function",
            "`lv.post(f, data)` + drain by the next `Lvgl::enter`",
            "calling `f` directly",
            pair(
                token,
                HEAVY,
                |t, n| {
                    for _ in 0..n {
                        let lv = enter(t);
                        lv.post(bump, black_box(1u32)).expect("post");
                    }
                    drop(enter(t));
                },
                |t, n| {
                    let mut lv = enter(t);
                    for _ in 0..n {
                        bump(&mut lv, black_box(1));
                    }
                },
            ),
        );
        row(
            "Timer handler, nothing pending",
            "`lv.timer_handler()`",
            "`lv_timer_handler()`",
            pair(
                token,
                CHEAP,
                |t, n| {
                    let mut lv = enter(t);
                    for _ in 0..n {
                        black_box(lv.timer_handler());
                    }
                },
                |t, n| {
                    let _lv = enter(t);
                    for _ in 0..n {
                        // SAFETY: session held.
                        black_box(unsafe { sys::lv_timer_handler() });
                    }
                },
            ),
        );
        rows
    }

    fn cell(stats: Option<Stats>) -> String {
        match stats {
            Some(s) => format!("{:.1} (min {:.1})", s.median, s.min),
            None => "0".into(),
        }
    }

    /// Table cells: closures' bars would end the cell.
    fn escape(text: &str) -> String {
        text.replace('|', "\\|")
    }

    fn performance_table(rows: &[Row]) -> String {
        let mut out = String::new();
        out.push_str(
            "| Operation | lvgl-rust | Raw `lvgl_rust::sys` | lvgl-rust, ns | Raw, ns | Difference, ns |\n",
        );
        out.push_str("| --- | --- | --- | ---: | ---: | ---: |\n");
        for r in rows {
            let raw_median = r.raw_stats.map_or(0.0, |s| s.median);
            let _ = writeln!(
                out,
                "| {} | {} | {} | {} | {} | {:+.1} |",
                escape(r.operation),
                escape(r.safe),
                escape(r.raw),
                cell(Some(r.safe_stats)),
                cell(r.raw_stats),
                r.safe_stats.median - raw_median
            );
        }
        out
    }

    /// Sizes of the handle types, checked against what the documentation
    /// states for 32-bit targets.
    fn types_table() -> String {
        let pointer = size_of::<usize>();
        // (type, size here, size on 32-bit targets, what it holds)
        let rows: [(&str, usize, usize, &str); 7] = [
            (
                "`Handle<W>`",
                size_of::<Handle<Obj>>(),
                4,
                "slot index and generation (`NonZeroU32`)",
            ),
            (
                "`Option<Handle<W>>`",
                size_of::<Option<Handle<Obj>>>(),
                4,
                "the same, through the niche",
            ),
            ("`Owned<W>`", size_of::<Owned<Obj>>(), 4, "a handle"),
            (
                "`Option<Owned<W>>`",
                size_of::<Option<Owned<Obj>>>(),
                4,
                "the same, through the niche",
            ),
            (
                "`ObjMut<'_, W>`",
                size_of::<ObjMut<'_, Obj>>(),
                4,
                "the object's pointer",
            ),
            (
                "`Session<'_>`, `Lvgl`",
                size_of::<Session<'_>>() + size_of::<Lvgl>(),
                0,
                "nothing: access is a type",
            ),
            (
                "`&Obj`, `&Label` (views)",
                size_of::<&Obj>(),
                4,
                "the object's pointer",
            ),
        ];
        assert_eq!(size_of::<Handle<Obj>>(), 4);
        assert_eq!(size_of::<Option<Handle<Obj>>>(), 4);
        assert_eq!(size_of::<Owned<Obj>>(), 4);
        assert_eq!(size_of::<Option<Owned<Obj>>>(), 4);
        assert_eq!(size_of::<ObjMut<'_, Obj>>(), pointer);
        assert_eq!(size_of::<Session<'_>>() + size_of::<Lvgl>(), 0);
        let mut out = String::new();
        let _ = writeln!(
            out,
            "| Type | Bytes on this host ({}-bit) | Bytes on 32-bit targets | Holds |",
            pointer * 8
        );
        out.push_str("| --- | ---: | ---: | --- |\n");
        for (name, here, embedded, holds) in rows {
            let _ = writeln!(out, "| {name} | {here} | {embedded} | {holds} |");
        }
        out
    }

    pub fn main() {
        let mut token = Token;
        let fx = {
            let mut lv = lvgl_rust::init(&mut token).expect("init");
            // Dropping a `Display` keeps the device.
            let _display = Display::create(
                &mut lv,
                &PANEL,
                NoPanel,
                64,
                64,
                DrawBuffers::single(&BUF),
                RenderMode::Partial,
            )
            .expect("display");
            Fixture::new(&mut lv)
        };
        let rows = rows(&mut token, &fx);
        assert!(HITS.load(Relaxed) > 0, "the handlers ran");

        let performance = performance_table(&rows);
        let types = types_table();
        println!("{performance}\n{types}");
        if let Some(dir) = std::env::var_os("LVGL_RUST_OVERHEAD_OUTPUT") {
            let dir = std::path::PathBuf::from(dir);
            std::fs::create_dir_all(&dir).expect("output directory");
            std::fs::write(dir.join("performance.md"), performance).expect("performance.md");
            std::fs::write(dir.join("types.md"), types).expect("types.md");
        }
    }
}
