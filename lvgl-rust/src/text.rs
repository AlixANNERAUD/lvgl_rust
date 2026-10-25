//! Text in and out of objects: strings LVGL returns from inside an object,
//! and label setters for `&str` and integers.

use crate::Error;
use core::ffi::{CStr, c_char};

/// Copies a string LVGL returned from inside an object into `buf`: the
/// object may free it on its next change, which a view cannot prevent.
/// `None` for NULL, [`Error::TooSmall`] if it does not fit with its NUL.
pub(crate) fn copy_str(p: *const c_char, buf: &mut [u8]) -> Result<Option<&CStr>, Error> {
    if p.is_null() {
        return Ok(None);
    }
    // SAFETY: a NUL-terminated string LVGL returned, valid during this call
    // (shared access: nothing runs in between).
    let text = unsafe { CStr::from_ptr(p) }.to_bytes_with_nul();
    let copy = buf.get_mut(..text.len()).ok_or(Error::TooSmall)?;
    copy.copy_from_slice(text);
    // SAFETY: a copy of a C string: one NUL, at the end.
    Ok(Some(unsafe { CStr::from_bytes_with_nul_unchecked(copy) }))
}

/// A string LVGL returned from inside an object, borrowed under exclusive
/// access (the caller ties `'a` to it).
pub(crate) fn borrow_str<'a>(p: *const c_char) -> Option<&'a CStr> {
    // SAFETY: NUL-terminated, and nothing can change the object while the
    // exclusive borrow lasts.
    (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) })
}

/// Decimal digits of `n` (with a sign if `negative`), NUL-terminated, at the
/// end of `buf`; returns where they start. Avoids `core::fmt`.
fn itoa(mut n: u32, negative: bool, buf: &mut [u8; 12]) -> usize {
    let mut i = buf.len() - 1;
    buf[i] = 0;
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    if negative {
        i -= 1;
        buf[i] = b'-';
    }
    i
}

#[cfg(lv_use_label)]
mod label {
    use super::itoa;
    use crate::{Error, sys, widgets::Label};

    /// Longest chunk inserted at once, plus its NUL.
    const CHUNK: usize = 32;

    impl Label {
        /// Sets the text from a `&str`, which LVGL copies. Fails with
        /// [`Error::InvalidArg`] on an interior NUL, and with
        /// [`Error::OutOfMemory`], leaving the label empty.
        ///
        /// LVGL takes C strings, so the text is appended in chunks of up to
        /// 31 bytes cut at character boundaries (`lv_label_ins_text`): use
        /// `set_text` with a `&CStr` for long texts.
        pub fn set_text_str(&self, text: &str) -> Result<(), Error> {
            if text.as_bytes().contains(&0) {
                return Err(Error::InvalidArg);
            }
            let mut appender = Appender::start(self.as_ptr())?;
            appender.push(text)?;
            appender.flush()
        }

        /// Sets the text to formatted output (`format_args!`), appended in
        /// chunks like [`set_text_str`](Label::set_text_str): no buffer
        /// beyond 32 bytes of stack. Fails with [`Error::InvalidArg`] on a
        /// NUL in the output, and with [`Error::OutOfMemory`].
        #[cfg(feature = "fmt")]
        pub fn set_text_fmt(&self, args: core::fmt::Arguments<'_>) -> Result<(), Error> {
            let mut appender = Appender::start(self.as_ptr())?;
            if core::fmt::write(&mut appender, args).is_err() {
                return Err(appender.error.unwrap_or(Error::InvalidArg));
            }
            appender.flush()
        }

        /// Sets the text to a signed number, without `core::fmt`.
        pub fn set_text_i32(&self, n: i32) {
            let mut buf = [0u8; 12];
            let start = itoa(n.unsigned_abs(), n < 0, &mut buf);
            self.set_digits(&buf, start);
        }

        /// Sets the text to an unsigned number, without `core::fmt`.
        pub fn set_text_u32(&self, n: u32) {
            let mut buf = [0u8; 12];
            let start = itoa(n, false, &mut buf);
            self.set_digits(&buf, start);
        }

        fn set_digits(&self, buf: &[u8; 12], start: usize) {
            if let Some(digits) = buf.get(start..) {
                // SAFETY: shared access; NUL-terminated, copied by LVGL. On
                // out of memory LVGL leaves the text NULL: make it valid.
                unsafe { sys::lv_label_set_text(self.as_ptr(), digits.as_ptr().cast()) };
                if !has_text(self.as_ptr()) {
                    empty(self.as_ptr());
                }
            }
        }
    }

    /// Appends text to a label in NUL-terminated chunks cut at character
    /// boundaries, because LVGL inserts at a character index.
    struct Appender {
        p: *mut sys::lv_obj_t,
        buf: [u8; CHUNK],
        len: usize,
        /// Why a formatted write stopped (`set_text_fmt`).
        #[cfg(feature = "fmt")]
        error: Option<Error>,
    }

    impl Appender {
        /// Empties the label (`lv_label_ins_text` does nothing on static
        /// text, `lv_label.c:727-728`).
        fn start(p: *mut sys::lv_obj_t) -> Result<Self, Error> {
            // SAFETY: shared access, a live label; LVGL copies the string.
            unsafe { sys::lv_label_set_text(p, c"".as_ptr()) };
            if !has_text(p) {
                return Err(empty(p));
            }
            Ok(Self {
                p,
                buf: [0; CHUNK],
                len: 0,
                #[cfg(feature = "fmt")]
                error: None,
            })
        }

        fn push(&mut self, mut text: &str) -> Result<(), Error> {
            if text.as_bytes().contains(&0) {
                return Err(Error::InvalidArg);
            }
            while !text.is_empty() {
                let space = CHUNK - 1 - self.len;
                // Back off to a character boundary (at most 3 bytes).
                let mut n = space.min(text.len());
                while !text.is_char_boundary(n) {
                    n -= 1;
                }
                if n == 0 {
                    self.flush()?;
                    continue;
                }
                let Some((chunk, tail)) = text.split_at_checked(n) else {
                    return Err(Error::InvalidArg); // unreachable: a boundary
                };
                if let Some(dst) = self.buf.get_mut(self.len..self.len + n) {
                    dst.copy_from_slice(chunk.as_bytes());
                }
                self.len += n;
                text = tail;
            }
            Ok(())
        }

        /// Inserts the buffered chunk.
        fn flush(&mut self) -> Result<(), Error> {
            if self.len == 0 {
                return Ok(());
            }
            if let Some(nul) = self.buf.get_mut(self.len) {
                *nul = 0;
            }
            self.len = 0;
            // SAFETY: as in `start`; `buf` is NUL-terminated.
            unsafe {
                sys::lv_label_ins_text(self.p, sys::LV_LABEL_POS_LAST, self.buf.as_ptr().cast())
            };
            if has_text(self.p) {
                Ok(())
            } else {
                Err(empty(self.p))
            }
        }
    }

    #[cfg(feature = "fmt")]
    impl core::fmt::Write for Appender {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            self.push(s).map_err(|error| {
                self.error = Some(error);
                core::fmt::Error
            })
        }
    }

    fn has_text(p: *mut sys::lv_obj_t) -> bool {
        // SAFETY: a live label, under the token.
        !unsafe { sys::lv_label_get_text(p) }.is_null()
    }

    /// After a failed allocation LVGL leaves the text NULL, which a later
    /// `lv_label_ins_text` would pass to `strlen`: point it at a static
    /// empty string, which never allocates.
    fn empty(p: *mut sys::lv_obj_t) -> Error {
        // SAFETY: a live label, under the token; the text is `'static`.
        unsafe { sys::lv_label_set_text_static(p, c"".as_ptr()) };
        Error::OutOfMemory
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn itoa_extremes() {
        let check = |n: u32, negative, expected: &CStr| {
            let mut buf = [0u8; 12];
            let start = itoa(n, negative, &mut buf);
            assert_eq!(CStr::from_bytes_with_nul(&buf[start..]), Ok(expected));
        };
        check(0, false, c"0");
        check(u32::MAX, false, c"4294967295");
        check(i32::MIN.unsigned_abs(), true, c"-2147483648");
        check(42, true, c"-42");
    }

    #[test]
    fn copies_or_reports_size() {
        let mut buf = [0u8; 4];
        assert_eq!(copy_str(c"abc".as_ptr(), &mut buf), Ok(Some(c"abc")));
        assert_eq!(copy_str(c"abcd".as_ptr(), &mut buf), Err(Error::TooSmall));
        assert_eq!(copy_str(core::ptr::null(), &mut buf), Ok(None));
    }
}
