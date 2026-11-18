//! The file half of FSM Editor's three editions: reading and writing a machine's two files, converting old (XMI)
//! files, validation, the text grammar, comparing and merging, code generation (through fsm-cli). Every function
//! takes texts and gives texts and data: nothing here reads or writes a file.

pub mod api;
pub mod compare;
pub mod convert;
pub mod format;
pub mod grammar;
pub mod merge;
pub mod model;
pub mod out;
pub mod paths;
pub mod validate;
pub mod xmi;
pub mod xml;

/// The WebAssembly interface: a request's bytes go into a buffer from `common_alloc`, `common_call` answers it (the
/// answer's length, little-endian u32, then its JSON) and frees the request, `common_free` frees the answer.
#[cfg(target_arch = "wasm32")]
mod wasm {
    /// A buffer of `len` bytes for the request.
    #[unsafe(no_mangle)]
    pub extern "C" fn common_alloc(len: usize) -> *mut u8 {
        Box::into_raw(vec![0u8; len].into_boxed_slice()) as *mut u8
    }

    /// Frees a buffer: an answer's `len` includes the 4 bytes of its length.
    ///
    /// # Safety
    /// `ptr` and `len` come from `common_alloc` or `common_call`; each buffer is freed once.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn common_free(ptr: *mut u8, len: usize) {
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) });
    }

    /// Answers the request in `ptr` and frees it.
    ///
    /// # Safety
    /// `ptr` and `len` come from `common_alloc`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn common_call(ptr: *mut u8, len: usize) -> *mut u8 {
        let request = unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) };
        let answer = crate::api::call(&request);
        drop(request);
        let mut out = Vec::with_capacity(4 + answer.len());
        out.extend_from_slice(&(answer.len() as u32).to_le_bytes());
        out.extend_from_slice(&answer);
        Box::into_raw(out.into_boxed_slice()) as *mut u8
    }
}
