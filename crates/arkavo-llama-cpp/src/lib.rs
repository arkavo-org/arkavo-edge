use arkavo_llama_cpp_sys as sys;
use std::ffi::CString;
use std::sync::Mutex;

pub struct LlamaModel {
    model: Mutex<*mut sys::llama_model>,
}

unsafe impl Send for LlamaModel {}
unsafe impl Sync for LlamaModel {}

impl LlamaModel {
    pub fn from_file(path: &str) -> Result<Self, String> {
        let c_path = CString::new(path).map_err(|e| e.to_string())?;
        let params = unsafe { sys::llama_model_default_params() };
        let model = unsafe { sys::llama_load_model_from_file(c_path.as_ptr(), params) };

        if model.is_null() {
            Err("Failed to load model".to_string())
        } else {
            Ok(Self {
                model: Mutex::new(model),
            })
        }
    }
}

impl Drop for LlamaModel {
    fn drop(&mut self) {
        unsafe {
            sys::llama_free_model(*self.model.lock().unwrap());
        }
    }
}

pub struct LlamaContext {
    context: Mutex<*mut sys::llama_context>,
}

unsafe impl Send for LlamaContext {}
unsafe impl Sync for LlamaContext {}

impl LlamaContext {
    pub fn new(model: &LlamaModel) -> Result<Self, String> {
        let params = unsafe { sys::llama_context_default_params() };
        let context =
            unsafe { sys::llama_new_context_with_model(*model.model.lock().unwrap(), params) };

        if context.is_null() {
            Err("Failed to create context".to_string())
        } else {
            Ok(Self {
                context: Mutex::new(context),
            })
        }
    }
}

impl Drop for LlamaContext {
    fn drop(&mut self) {
        unsafe {
            sys::llama_free(*self.context.lock().unwrap());
        }
    }
}
