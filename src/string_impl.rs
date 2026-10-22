// MIT License
//
// Copyright (c) 2018 Redox OS
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

use core::{
    ffi::{c_char, c_int},
    ptr,
};
#[allow(non_camel_case_types)]
type size_t = usize;

#[no_mangle]
pub unsafe extern "C" fn strchr(mut s: *const c_char, c: c_int) -> *mut c_char {
    // The terminating NUL is part of the string, so `strchr(s, 0)` finds it.
    let c = c as c_char;
    loop {
        if *s == c {
            return s as *mut c_char;
        }
        if *s == 0 {
            return ptr::null_mut();
        }
        s = s.add(1);
    }
}

#[no_mangle]
pub unsafe extern "C" fn strcmp(s1: *const c_char, s2: *const c_char) -> c_int {
    strncmp(s1, s2, usize::MAX)
}

#[no_mangle]
pub unsafe extern "C" fn strcoll(s1: *const c_char, s2: *const c_char) -> c_int {
    // relibc has no locale stuff (yet)
    strcmp(s1, s2)
}

#[no_mangle]
pub unsafe extern "C" fn strcpy(dst: *mut c_char, src: *const c_char) -> *mut c_char {
    let mut i = 0;

    loop {
        let byte = *src.offset(i);
        *dst.offset(i) = byte;

        if byte == 0 {
            break;
        }

        i += 1;
    }

    dst
}

#[no_mangle]
pub unsafe extern "C" fn strlen(s: *const c_char) -> size_t {
    strnlen(s, usize::MAX)
}

#[no_mangle]
pub unsafe extern "C" fn strnlen(s: *const c_char, size: size_t) -> size_t {
    let mut i = 0;
    while i < size {
        if *s.add(i) == 0 {
            break;
        }
        i += 1;
    }
    i as size_t
}

#[no_mangle]
pub unsafe extern "C" fn strnlen_s(s: *const c_char, size: size_t) -> size_t {
    if s.is_null() {
        0
    } else {
        strnlen(s, size)
    }
}

#[no_mangle]
pub unsafe extern "C" fn strcat(s1: *mut c_char, s2: *const c_char) -> *mut c_char {
    strncat(s1, s2, usize::MAX)
}

#[no_mangle]
pub unsafe extern "C" fn strncat(s1: *mut c_char, s2: *const c_char, n: size_t) -> *mut c_char {
    let len = strlen(s1 as *const c_char);
    let mut i = 0;
    while i < n {
        let b = *s2.add(i);
        if b == 0 {
            break;
        }

        *s1.add(len + i) = b;
        i += 1;
    }
    *s1.add(len + i) = 0;

    s1
}

#[no_mangle]
pub unsafe extern "C" fn strncmp(s1: *const c_char, s2: *const c_char, n: size_t) -> c_int {
    let mut i = 0;
    while i < n {
        // C compares characters as `unsigned char` (`c_char` is signed on
        // some targets).
        #[allow(clippy::unnecessary_cast)]
        let (c1, c2) = (*s1.add(i) as u8, *s2.add(i) as u8);
        if c1 != c2 || c1 == 0 {
            return (c1 as c_int) - (c2 as c_int);
        }
        i += 1;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn strncpy(dst: *mut c_char, src: *const c_char, n: size_t) -> *mut c_char {
    let mut i = 0;

    while i < n && *src.add(i) != 0 {
        *dst.add(i) = *src.add(i);
        i += 1;
    }

    for i in i..n {
        *dst.add(i) = 0;
    }

    dst
}

#[no_mangle]
pub unsafe extern "C" fn strrchr(s: *const c_char, c: c_int) -> *mut c_char {
    // Searching includes the terminating NUL, like `strchr`.
    let c = c as c_char;
    let mut i = strlen(s) + 1;
    while i > 0 {
        i -= 1;
        if *s.add(i) == c {
            return s.add(i) as *mut c_char;
        }
    }
    ptr::null_mut()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// NUL-terminated C string literal as a `*const c_char`.
    macro_rules! c {
        ($s:literal) => {
            concat!($s, "\0").as_ptr() as *const c_char
        };
    }

    unsafe fn offset_of(base: *const c_char, found: *mut c_char) -> Option<usize> {
        (!found.is_null()).then(|| found.offset_from(base) as usize)
    }

    #[test]
    fn strcmp_test() {
        unsafe {
            let s1 = [1, 2, 0].as_ptr();
            let s2 = [1, 2, 3, 0].as_ptr();
            assert!(strncmp(s1, s2, 0) == 0);
            assert!(strncmp(s1, s2, 1) == 0);
            assert!(strncmp(s1, s2, 2) == 0);
            assert!(strncmp(s1, s2, 3) < 0);
            assert!(strncmp(s2, s1, 3) > 0);
            assert!(strncmp(s1, s2, 4) < 0);
            assert!(strncmp(s2, s1, 4) > 0);
            assert!(strcmp(s1, s2) < 0);
            assert!(strcmp(s2, s1) > 0);
            assert_eq!(strcmp(c!("lvgl"), c!("lvgl")), 0);
            assert_eq!(strcoll(c!("a"), c!("b")), strcmp(c!("a"), c!("b")));
        }
    }

    #[test]
    fn strcmp_compares_bytes_as_unsigned() {
        unsafe {
            // "é" is 0xC3 0xA9 in UTF-8 and must sort after ASCII.
            assert!(strcmp(c!("é"), c!("z")) > 0);
            assert!(strncmp(c!("z"), c!("é"), 1) < 0);
        }
    }

    #[test]
    fn strlen_and_strnlen() {
        unsafe {
            assert_eq!(strlen(c!("")), 0);
            assert_eq!(strlen(c!("hello")), 5);
            assert_eq!(strnlen(c!("hello"), 3), 3);
            assert_eq!(strnlen(c!("hello"), 10), 5);
            assert_eq!(strnlen_s(ptr::null(), 10), 0);
            assert_eq!(strnlen_s(c!("hi"), 10), 2);
        }
    }

    #[test]
    fn strchr_and_strrchr() {
        unsafe {
            let s = c!("a/b/c");
            assert_eq!(offset_of(s, strchr(s, b'/' as c_int)), Some(1));
            assert_eq!(offset_of(s, strrchr(s, b'/' as c_int)), Some(3));
            assert_eq!(offset_of(s, strchr(s, b'x' as c_int)), None);
            assert_eq!(offset_of(s, strrchr(s, b'x' as c_int)), None);
            assert_eq!(offset_of(s, strchr(s, 0)), Some(5));
            assert_eq!(offset_of(s, strrchr(s, 0)), Some(5));
            assert_eq!(offset_of(c!(""), strrchr(c!(""), b'a' as c_int)), None);
        }
    }

    #[test]
    fn strcpy_and_strcat() {
        unsafe {
            let mut buffer = [0x55 as c_char; 16];
            let dst = buffer.as_mut_ptr();
            assert_eq!(strcpy(dst, c!("lv")), dst);
            assert_eq!(strcmp(dst, c!("lv")), 0);
            assert_eq!(strcat(dst, c!("gl")), dst);
            assert_eq!(strcmp(dst, c!("lvgl")), 0);
            strncat(dst, c!("_rust"), 3);
            assert_eq!(strcmp(dst, c!("lvgl_ru")), 0);
        }
    }

    #[test]
    fn strncpy_pads_and_stops_at_limit() {
        unsafe {
            let mut buffer = [0x55 as c_char; 6];
            strncpy(buffer.as_mut_ptr(), c!("ab"), 5);
            assert_eq!(buffer, [b'a' as c_char, b'b' as c_char, 0, 0, 0, 0x55]);

            // Not NUL-terminated within `n`: copies exactly `n` bytes and
            // never reads past them.
            let src = [b'x' as c_char, b'y' as c_char];
            let mut buffer = [0x55 as c_char; 3];
            strncpy(buffer.as_mut_ptr(), src.as_ptr(), 2);
            assert_eq!(buffer, [b'x' as c_char, b'y' as c_char, 0x55]);
        }
    }
}
