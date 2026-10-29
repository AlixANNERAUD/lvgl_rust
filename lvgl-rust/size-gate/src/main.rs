//! Firmware for the size gate (`mise run size-gate`): uses the core API so
//! that the linked image contains what an application pays for. Built for
//! `thumbv6m-none-eabi` and `riscv32imc-unknown-none-elf`; on other targets
//! it is an empty program.

#![cfg_attr(target_os = "none", no_std, no_main)]

#[cfg(target_os = "none")]
mod firmware {
    use lvgl_rust::{Lvgl, LvglAccess, Obj, ScreenAnim, init, tick_inc};

    struct Guard;

    // SAFETY: single-threaded firmware, one guard.
    unsafe impl LvglAccess for Guard {}

    /// A bump allocator over a static block, for `growable-registry`
    /// (chunks are never freed). Single-threaded firmware: plain cells.
    #[cfg(feature = "growable-registry")]
    mod allocator {
        use core::alloc::{GlobalAlloc, Layout};
        use core::cell::{Cell, UnsafeCell};

        struct Bump {
            block: UnsafeCell<[u8; 4096]>,
            used: Cell<usize>,
        }

        // SAFETY: single-threaded firmware.
        unsafe impl Sync for Bump {}

        unsafe impl GlobalAlloc for Bump {
            unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
                // Wrapping arithmetic: no panic path in the firmware.
                let base = self.block.get().cast::<u8>();
                let mask = layout.align().wrapping_sub(1);
                let start = ((base as usize)
                    .wrapping_add(self.used.get())
                    .wrapping_add(mask)
                    & !mask)
                    .wrapping_sub(base as usize);
                let end = start.wrapping_add(layout.size());
                if end > 4096 || end < start {
                    return core::ptr::null_mut();
                }
                self.used.set(end);
                // SAFETY: within the block.
                unsafe { base.add(start) }
            }

            unsafe fn dealloc(&self, _: *mut u8, _: Layout) {}
        }

        #[global_allocator]
        static ALLOCATOR: Bump = Bump {
            block: UnsafeCell::new([0; 4096]),
            used: Cell::new(0),
        };
    }

    #[panic_handler]
    fn panic(_: &core::panic::PanicInfo) -> ! {
        loop {}
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn _start() -> ! {
        let mut guard = Guard;
        let _ = run(&mut guard);
        // What reading the counters costs (the static stays linked even
        // where nothing below counts).
        #[cfg(feature = "debug-counters")]
        core::hint::black_box(lvgl_rust::DebugCounters::read());
        loop {
            tick_inc(1);
            if let Ok(mut lv) = Lvgl::enter(&mut guard) {
                lv.timer_handler();
            }
        }
    }

    #[cfg(not(lvgl_rust_views_only))]
    fn run(guard: &mut Guard) -> Result<(), lvgl_rust::Error> {
        use lvgl_rust::WidgetExt;
        let mut lv = init(guard)?;
        let screen = lv.create_screen()?;
        let panel = Obj::create(&mut screen.get_mut(&mut lv)?)?.handle()?;
        let child = Obj::create(&mut panel.get_mut(&mut lv)?)?.handle()?;
        panel.get(&lv)?.clean_later()?;
        let owned = Obj::create_owned(&mut screen.get_mut(&mut lv)?)?;
        lv.load_screen(screen.handle(), ScreenAnim::NONE)?;
        child.get_mut(&mut lv)?.delete()?;
        drop(owned);
        lv.timer_handler();
        core::mem::forget(screen);
        Ok(())
    }

    #[cfg(lvgl_rust_views_only)]
    fn run(guard: &mut Guard) -> Result<(), lvgl_rust::Error> {
        let mut lv = init(guard)?;
        // A panel in a new screen and one in the active screen, created
        // through the screen's `ObjMut`, which then cleans and loads it.
        Obj::create(&mut lv.create_screen()?)?;
        let mut screen = lv.screen_active_mut().ok_or(lvgl_rust::Error::Dead)?;
        Obj::create(&mut screen)?;
        screen.clean()?;
        screen.load_screen(ScreenAnim::NONE)?;
        lv.timer_handler();
        Ok(())
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
