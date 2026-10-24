//! Kleine Hilfen für Unix-Systeme mit selbst geschriebener FFI.

use core::ffi::{c_char, c_int};

unsafe extern "C" {
    /// POSIX: `int gethostname(char *name, size_t len)`. `size_t` entspricht
    /// auf allen Zielen von Rust `usize`.
    fn gethostname(name: *mut c_char, len: usize) -> c_int;
}

/// Der Rechnername, wie ihn `gethostname` liefert, ohne abschließende Null.
pub fn hostname() -> Option<Vec<u8>> {
    let mut buffer = [0u8; 256];
    // SAFETY: Der Puffer gehört uns und ist `buffer.len()` Bytes lang;
    // gethostname schreibt höchstens so viele Bytes hinein.
    let result = unsafe { gethostname(buffer.as_mut_ptr().cast::<c_char>(), buffer.len()) };
    if result != 0 {
        return None;
    }
    // Bei Kürzung ist das Ergebnis laut POSIX nicht unbedingt nullterminiert.
    let length = buffer.iter().position(|&b| b == 0).unwrap_or(buffer.len());
    buffer.get(..length).map(<[u8]>::to_vec)
}

#[cfg(test)]
mod tests {
    #[test]
    fn hostname_is_available() {
        let name = super::hostname().expect("gethostname");
        assert!(!name.contains(&0));
    }
}
