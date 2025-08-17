pub use arkavo_llama_cpp_sys as ffi;

use std::ffi::CString;
use std::os::raw::c_char;

pub struct LlamaModel {
    pub(crate) ptr: *mut ffi::llama_model,
}

// SAFETY: llama.cpp's model objects are thread-safe for read operations
unsafe impl Send for LlamaModel {}
unsafe impl Sync for LlamaModel {}

impl LlamaModel {
    pub fn from_file(path: &str) -> Result<Self, String> {
        let c_path = CString::new(path).unwrap();
        let params = unsafe { ffi::llama_model_default_params() };
        let model = unsafe { ffi::llama_load_model_from_file(c_path.as_ptr(), params) };
        if model.is_null() {
            Err("Failed to load model".to_string())
        } else {
            Ok(Self { ptr: model })
        }
    }

    pub fn get_vocab(&self) -> *const ffi::llama_vocab {
        unsafe { ffi::llama_model_get_vocab(self.ptr) }
    }
}

impl Drop for LlamaModel {
    fn drop(&mut self) {
        unsafe {
            ffi::llama_free_model(self.ptr);
        }
    }
}

pub struct LlamaContext {
    pub(crate) ptr: *mut ffi::llama_context,
}

// SAFETY: llama.cpp contexts need to be protected by mutex for thread safety
unsafe impl Send for LlamaContext {}

impl LlamaContext {
    pub fn new(model: &LlamaModel) -> Result<Self, String> {
        let params = unsafe { ffi::llama_context_default_params() };
        let context = unsafe { ffi::llama_new_context_with_model(model.ptr, params) };
        if context.is_null() {
            Err("Failed to create context".to_string())
        } else {
            Ok(Self { ptr: context })
        }
    }
}

impl Drop for LlamaContext {
    fn drop(&mut self) {
        unsafe {
            ffi::llama_free(self.ptr);
        }
    }
}

pub fn apply_chat_template(
    messages: &[ffi::llama_chat_message],
    add_assistant: bool,
) -> Result<Vec<u8>, String> {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let wrote = unsafe {
            ffi::llama_chat_apply_template(
                std::ptr::null(),              // NULL => use model's default chat template
                messages.as_ptr(),
                messages.len(),
                add_assistant,
                buf.as_mut_ptr() as *mut c_char,
                buf.len() as i32,
            )
        };
        if wrote >= 0 && (wrote as usize) <= buf.len() {
            buf.truncate(wrote as usize);
            return Ok(buf);
        }
        let need = wrote.checked_neg().unwrap_or(128 * 1024) as usize;
        buf.resize(need, 0);
    }
}

pub fn tokenize_with_model(
    vocab: *const ffi::llama_vocab,
    text_utf8: &[u8],
) -> Result<Vec<ffi::llama_token>, String> {
    let mut toks = vec![0i32; text_utf8.len() + 8];
    loop {
        let n = unsafe {
            ffi::llama_tokenize(
                vocab,
                text_utf8.as_ptr() as *const c_char,
                text_utf8.len() as i32,
                toks.as_mut_ptr(),
                toks.len() as i32,
                true,   // add_special (BOS/EOS if appropriate)
                true,   // parse_special (chat template control tokens)
            )
        };
        if n >= 0 && (n as usize) <= toks.len() {
            toks.truncate(n as usize);
            return Ok(toks);
        }
        let need = n.checked_neg().unwrap_or((toks.len() * 2) as i32) as usize;
        toks.resize(need, 0);
    }
}

pub fn detokenize(
    vocab: *const ffi::llama_vocab,
    tokens: &[ffi::llama_token],
    remove_special: bool,
    unparse_special: bool,
) -> Result<String, String> {
    let mut buf = vec![0u8; tokens.len() * 8 + 16];
    loop {
        let n = unsafe {
            ffi::llama_detokenize(
                vocab,
                tokens.as_ptr(),
                tokens.len() as i32,
                buf.as_mut_ptr() as *mut c_char,
                buf.len() as i32,
                remove_special,
                unparse_special,
            )
        };
        if n >= 0 && (n as usize) <= buf.len() {
            buf.truncate(n as usize);
            return String::from_utf8(buf).map_err(|e| format!("UTF-8 conversion error: {}", e));
        }
        let need = n.checked_neg().unwrap_or((buf.len() * 2) as i32) as usize;
        buf.resize(need, 0);
    }
}