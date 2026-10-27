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

    #[panic_handler]
    fn panic(_: &core::panic::PanicInfo) -> ! {
        loop {}
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn _start() -> ! {
        let mut guard = Guard;
        let _ = run(&mut guard);
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
        Obj::create(&mut lv.create_screen()?)?;
        Obj::create(&mut lv.screen_active_mut().ok_or(lvgl_rust::Error::Dead)?)?;
        Obj::create(
            &mut lv
                .edit(|lv| lv.screen_active()?.child(0))
                .ok_or(lvgl_rust::Error::Dead)?,
        )?;
        lv.edit(|lv| lv.screen_active()?.child(0))
            .ok_or(lvgl_rust::Error::Dead)?
            .clean()?;
        lv.edit(|lv| lv.screen_active())
            .ok_or(lvgl_rust::Error::Dead)?
            .load_screen(ScreenAnim::NONE)?;
        lv.timer_handler();
        Ok(())
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
