//! Virtual GGUF reader as a stdio `FILE*` for `llama_model_load_from_file_ptr`.
//!
//! `funopen` (BSD/macOS) or `fopencookie` (Linux) so llama.cpp never sees TDF/zip/AES.

use std::os::raw::{c_char, c_int, c_void};

struct Cookie {
    // Fat pointer to the caller's `FnMut`; lifetime is guaranteed by
    // `StdioCookieFile` not outliving `from_callback`'s stack frame.
    read_at: *mut (dyn FnMut(u64, &mut [u8]) -> usize + 'static),
    virtual_size: u64,
    pos: u64,
}

pub(crate) struct StdioCookieFile {
    file: *mut libc::FILE,
    _cookie: Box<Cookie>,
}

impl Drop for StdioCookieFile {
    fn drop(&mut self) {
        if !self.file.is_null() {
            // SAFETY: `file` is a cookie stream we created; llama.cpp does not fclose it.
            unsafe {
                libc::fclose(self.file);
            }
            self.file = std::ptr::null_mut();
        }
    }
}

impl StdioCookieFile {
    pub(crate) fn open(
        virtual_size: u64,
        read_at: &mut dyn FnMut(u64, &mut [u8]) -> usize,
    ) -> Result<Self, String> {
        let read_at_ptr: *mut dyn FnMut(u64, &mut [u8]) -> usize = read_at;
        let mut cookie = Box::new(Cookie {
            // SAFETY: StdioCookieFile is dropped before `read_at` on the caller stack.
            read_at: unsafe {
                std::mem::transmute::<
                    *mut dyn FnMut(u64, &mut [u8]) -> usize,
                    *mut (dyn FnMut(u64, &mut [u8]) -> usize + 'static),
                >(read_at_ptr)
            },
            virtual_size,
            pos: 0,
        });
        let file = unsafe { open_cookie_file(cookie.as_mut()) };
        if file.is_null() {
            return Err("failed to create stdio cookie FILE* for callback reader".to_string());
        }
        Ok(Self {
            file,
            _cookie: cookie,
        })
    }

    pub(crate) fn as_ptr(&self) -> *mut crate::ffi::FILE {
        self.file.cast()
    }

    pub(crate) fn rewind(&self) -> Result<(), String> {
        // SAFETY: `file` is our cookie stream; fseek invokes cookie_seek.
        let rc = unsafe { libc::fseek(self.file, 0, libc::SEEK_SET) };
        if rc != 0 {
            Err("failed to rewind callback FILE*".to_string())
        } else {
            Ok(())
        }
    }
}

unsafe fn open_cookie_file(cookie: *mut Cookie) -> *mut libc::FILE {
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly",
        target_os = "android"
    ))]
    {
        funopen(
            cookie.cast(),
            Some(cookie_read_bsd),
            None,
            Some(cookie_seek_bsd),
            None,
        )
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        let io = CookieIoFunctions {
            read: Some(cookie_read_gnu),
            write: None,
            seek: Some(cookie_seek_gnu),
            close: None,
        };
        fopencookie(cookie.cast(), c"r".as_ptr(), io)
    }

    #[cfg(not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly",
        target_os = "android",
        all(target_os = "linux", target_env = "gnu")
    )))]
    {
        let _ = cookie;
        std::ptr::null_mut()
    }
}

fn apply_seek(cookie: &mut Cookie, offset: i64, whence: c_int) -> Option<u64> {
    let base = match whence {
        libc::SEEK_SET => 0i128,
        libc::SEEK_CUR => cookie.pos as i128,
        libc::SEEK_END => cookie.virtual_size as i128,
        _ => return None,
    };
    let pos = base.checked_add(i128::from(offset))?;
    if pos < 0 {
        return None;
    }
    Some(pos as u64)
}

fn read_from_cookie(cookie: &mut Cookie, buf: &mut [u8]) -> usize {
    if buf.is_empty() || cookie.pos >= cookie.virtual_size {
        return 0;
    }
    let max = (cookie.virtual_size - cookie.pos) as usize;
    let n = buf.len().min(max);
    let dest = &mut buf[..n];
    // SAFETY: `read_at` points at the closure held on the `from_callback` stack,
    // which outlives this FILE* (StdioCookieFile is dropped before the closure).
    let got = unsafe { (*cookie.read_at)(cookie.pos, dest) };
    let got = got.min(dest.len());
    cookie.pos += got as u64;
    got
}

unsafe extern "C" fn cookie_read_bsd(cookie: *mut c_void, buf: *mut c_char, n: c_int) -> c_int {
    if cookie.is_null() || buf.is_null() || n <= 0 {
        return 0;
    }
    let cookie = unsafe { &mut *cookie.cast::<Cookie>() };
    let slice = unsafe { std::slice::from_raw_parts_mut(buf.cast::<u8>(), n as usize) };
    read_from_cookie(cookie, slice) as c_int
}

unsafe extern "C" fn cookie_seek_bsd(
    cookie: *mut c_void,
    offset: libc::off_t,
    whence: c_int,
) -> libc::off_t {
    if cookie.is_null() {
        return -1;
    }
    let cookie = unsafe { &mut *cookie.cast::<Cookie>() };
    match apply_seek(cookie, offset, whence) {
        Some(pos) => {
            cookie.pos = pos;
            pos as libc::off_t
        }
        None => -1,
    }
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
unsafe extern "C" fn cookie_read_gnu(cookie: *mut c_void, buf: *mut c_char, n: usize) -> isize {
    if cookie.is_null() || buf.is_null() || n == 0 {
        return 0;
    }
    let cookie = unsafe { &mut *cookie.cast::<Cookie>() };
    let slice = unsafe { std::slice::from_raw_parts_mut(buf.cast::<u8>(), n) };
    read_from_cookie(cookie, slice) as isize
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
unsafe extern "C" fn cookie_seek_gnu(
    cookie: *mut c_void,
    offset: *mut i64,
    whence: c_int,
) -> c_int {
    if cookie.is_null() || offset.is_null() {
        return -1;
    }
    let cookie = unsafe { &mut *cookie.cast::<Cookie>() };
    let off = unsafe { *offset };
    match apply_seek(cookie, off, whence) {
        Some(pos) => {
            cookie.pos = pos;
            unsafe {
                *offset = pos as i64;
            }
            0
        }
        None => -1,
    }
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly",
    target_os = "android"
))]
unsafe extern "C" {
    fn funopen(
        cookie: *mut c_void,
        readfn: Option<unsafe extern "C" fn(*mut c_void, *mut c_char, c_int) -> c_int>,
        writefn: Option<unsafe extern "C" fn(*mut c_void, *const c_char, c_int) -> c_int>,
        seekfn: Option<unsafe extern "C" fn(*mut c_void, libc::off_t, c_int) -> libc::off_t>,
        closefn: Option<unsafe extern "C" fn(*mut c_void) -> c_int>,
    ) -> *mut libc::FILE;
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[repr(C)]
struct CookieIoFunctions {
    read: Option<unsafe extern "C" fn(*mut c_void, *mut c_char, usize) -> isize>,
    write: Option<unsafe extern "C" fn(*mut c_void, *const c_char, usize) -> isize>,
    seek: Option<unsafe extern "C" fn(*mut c_void, *mut i64, c_int) -> c_int>,
    close: Option<unsafe extern "C" fn(*mut c_void) -> c_int>,
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
unsafe extern "C" {
    fn fopencookie(
        cookie: *mut c_void,
        mode: *const c_char,
        io: CookieIoFunctions,
    ) -> *mut libc::FILE;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_file_seek_end_and_read() {
        let data = b"hello world!!";
        let mut read_at = |offset: u64, buf: &mut [u8]| -> usize {
            let start = offset as usize;
            if start >= data.len() {
                return 0;
            }
            let n = buf.len().min(data.len() - start);
            buf[..n].copy_from_slice(&data[start..start + n]);
            n
        };
        let file = StdioCookieFile::open(data.len() as u64, &mut read_at).unwrap();
        unsafe {
            assert_eq!(libc::fseek(file.file, 0, libc::SEEK_END), 0);
            assert_eq!(libc::ftell(file.file), data.len() as libc::c_long);
            assert_eq!(libc::fseek(file.file, 0, libc::SEEK_SET), 0);
            let mut buf = [0u8; 5];
            let n = libc::fread(buf.as_mut_ptr().cast(), 1, 5, file.file);
            assert_eq!(n, 5);
            assert_eq!(&buf, b"hello");
            assert_eq!(libc::fseek(file.file, 6, libc::SEEK_SET), 0);
            let mut buf2 = [0u8; 5];
            let n = libc::fread(buf2.as_mut_ptr().cast(), 1, 5, file.file);
            assert_eq!(n, 5);
            assert_eq!(&buf2, b"world");
        }
    }
}
