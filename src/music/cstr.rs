//! Compile-time `&'static CStr` construction for chord labels.

/// Build a `&'static CStr` from a string literal at compile time.
///
/// Chord labels and quality suffixes are handed to C hosts as static pointers
/// the host never frees, so they must be nul-terminated. Rust's own `c"..."`
/// literals would be tidier, but the cbindgen build script parses this crate's
/// source with `syn` 1.x, which cannot tokenize them. The inner `const` item
/// forces evaluation — and therefore the interior-nul check — at compile time.
macro_rules! cstr {
    ($text:literal) => {{
        const VALUE: &::std::ffi::CStr =
            match ::std::ffi::CStr::from_bytes_with_nul(concat!($text, "\0").as_bytes()) {
                Ok(value) => value,
                Err(_) => panic!("cstr! text must not contain interior nul bytes"),
            };
        VALUE
    }};
}
pub(crate) use cstr;
