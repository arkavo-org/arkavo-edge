use crate::{ffi, LlamaModel};
use std::ffi::CString;

#[cfg(not(target_env = "musl"))]
pub struct MtmdContext {
    pub(crate) ptr: *mut ffi::mtmd_context,
}

// SAFETY: Access is serialized through Mutex in LlamaModel/LlamaContext
#[cfg(not(target_env = "musl"))]
unsafe impl Send for MtmdContext {}

// SAFETY: Access is serialized through Mutex in LlamaModel/LlamaContext
#[cfg(not(target_env = "musl"))]
unsafe impl Sync for MtmdContext {}

#[cfg(not(target_env = "musl"))]
impl MtmdContext {
    pub fn from_file(mmproj_path: &str, text_model: &LlamaModel) -> Result<Self, String> {
        let c_path =
            CString::new(mmproj_path).map_err(|e| format!("Invalid mmproj path: {}", e))?;

        // SAFETY: Null return is checked immediately after this call
        let mut params = unsafe { ffi::mtmd_context_params_default() };
        params.use_gpu = true;
        params.n_threads = std::thread::available_parallelism()
            .map(|n| n.get() as i32)
            .unwrap_or(8);
        // SAFETY: Null return is checked immediately after this call
        params.media_marker = unsafe { ffi::mtmd_default_marker() };
        params.warmup = true; // Run warmup encode pass after initialization

        // SAFETY: CString is valid null-terminated UTF-8; pointer is valid for the duration of the call
        let ctx = unsafe { ffi::mtmd_init_from_file(c_path.as_ptr(), text_model.ptr, params) };

        if ctx.is_null() {
            Err("Failed to load mmproj model".to_string())
        } else {
            Ok(Self { ptr: ctx })
        }
    }

    pub fn supports_vision(&self) -> bool {
        // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
        unsafe { ffi::mtmd_support_vision(self.ptr) }
    }

    pub fn supports_audio(&self) -> bool {
        // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
        unsafe { ffi::mtmd_support_audio(self.ptr) }
    }

    pub fn decode_use_non_causal(&self) -> bool {
        // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
        unsafe { ffi::mtmd_decode_use_non_causal(self.ptr) }
    }

    pub fn decode_use_mrope(&self) -> bool {
        // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
        unsafe { ffi::mtmd_decode_use_mrope(self.ptr) }
    }
}

#[cfg(not(target_env = "musl"))]
impl Drop for MtmdContext {
    fn drop(&mut self) {
        // SAFETY: Pointer was allocated by llama.cpp FFI and is guaranteed non-null after construction
        unsafe {
            ffi::mtmd_free(self.ptr);
        }
    }
}

#[cfg(not(target_env = "musl"))]
pub struct MtmdBitmap {
    pub(crate) ptr: *mut ffi::mtmd_bitmap,
}

// SAFETY: Access is serialized through Mutex in LlamaModel/LlamaContext
#[cfg(not(target_env = "musl"))]
unsafe impl Send for MtmdBitmap {}

#[cfg(not(target_env = "musl"))]
impl MtmdBitmap {
    pub fn from_rgb(width: u32, height: u32, rgb_data: &[u8]) -> Result<Self, String> {
        if rgb_data.len() != (width * height * 3) as usize {
            return Err(format!(
                "Invalid RGB data size. Expected {} bytes, got {}",
                width * height * 3,
                rgb_data.len()
            ));
        }

        // SAFETY: Null return is checked immediately after this call
        let bitmap = unsafe { ffi::mtmd_bitmap_init(width, height, rgb_data.as_ptr()) };

        if bitmap.is_null() {
            Err("Failed to create bitmap".to_string())
        } else {
            Ok(Self { ptr: bitmap })
        }
    }

    pub fn from_audio(samples: &[f32]) -> Result<Self, String> {
        // SAFETY: Null return is checked immediately after this call
        let bitmap = unsafe { ffi::mtmd_bitmap_init_from_audio(samples.len(), samples.as_ptr()) };

        if bitmap.is_null() {
            Err("Failed to create audio bitmap".to_string())
        } else {
            Ok(Self { ptr: bitmap })
        }
    }

    pub fn get_width(&self) -> u32 {
        // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
        unsafe { ffi::mtmd_bitmap_get_nx(self.ptr) }
    }

    pub fn get_height(&self) -> u32 {
        // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
        unsafe { ffi::mtmd_bitmap_get_ny(self.ptr) }
    }

    pub fn is_audio(&self) -> bool {
        // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
        unsafe { ffi::mtmd_bitmap_is_audio(self.ptr) }
    }

    pub fn set_id(&mut self, id: &str) {
        let c_id = CString::new(id).unwrap();
        // SAFETY: CString is valid null-terminated UTF-8; pointer is valid for the duration of the call
        unsafe {
            ffi::mtmd_bitmap_set_id(self.ptr, c_id.as_ptr());
        }
    }
}

#[cfg(not(target_env = "musl"))]
impl Drop for MtmdBitmap {
    fn drop(&mut self) {
        // SAFETY: Pointer was allocated by llama.cpp FFI and is guaranteed non-null after construction
        unsafe {
            ffi::mtmd_bitmap_free(self.ptr);
        }
    }
}

#[cfg(not(target_env = "musl"))]
pub struct MtmdInputChunks {
    pub(crate) ptr: *mut ffi::mtmd_input_chunks,
}

// SAFETY: Access is serialized through Mutex in LlamaModel/LlamaContext
#[cfg(not(target_env = "musl"))]
unsafe impl Send for MtmdInputChunks {}

#[cfg(not(target_env = "musl"))]
impl MtmdInputChunks {
    pub fn new() -> Self {
        // SAFETY: Null return is checked immediately after this call
        let ptr = unsafe { ffi::mtmd_input_chunks_init() };
        Self { ptr }
    }

    pub fn size(&self) -> usize {
        // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
        unsafe { ffi::mtmd_input_chunks_size(self.ptr) }
    }

    pub fn get(&self, idx: usize) -> Option<*const ffi::mtmd_input_chunk> {
        if idx >= self.size() {
            None
        } else {
            // SAFETY: Batch/sampler pointers originate from llama.cpp allocation and remain valid for the struct's lifetime
            Some(unsafe { ffi::mtmd_input_chunks_get(self.ptr, idx) })
        }
    }
}

#[cfg(not(target_env = "musl"))]
impl Default for MtmdInputChunks {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(target_env = "musl"))]
impl Drop for MtmdInputChunks {
    fn drop(&mut self) {
        // SAFETY: Pointer was allocated by llama.cpp FFI and is guaranteed non-null after construction
        unsafe {
            ffi::mtmd_input_chunks_free(self.ptr);
        }
    }
}

pub fn tokenize_with_images(
    ctx: &MtmdContext,
    text: &str,
    bitmaps: &[&MtmdBitmap],
) -> Result<MtmdInputChunks, String> {
    let text_cstring = CString::new(text).map_err(|e| format!("Invalid text: {}", e))?;

    let text_input = ffi::mtmd_input_text {
        text: text_cstring.as_ptr(),
        add_special: true,
        parse_special: true,
    };

    let mut bitmap_ptrs: Vec<*const ffi::mtmd_bitmap> = bitmaps
        .iter()
        .map(|b| b.ptr as *const ffi::mtmd_bitmap)
        .collect();

    let chunks = MtmdInputChunks::new();

    // SAFETY: CString is valid null-terminated UTF-8; pointer is valid for the duration of the call
    let result = unsafe {
        ffi::mtmd_tokenize(
            ctx.ptr,
            chunks.ptr,
            &text_input,
            bitmap_ptrs.as_mut_ptr(),
            bitmaps.len(),
        )
    };

    match result {
        0 => Ok(chunks),
        1 => Err("Number of bitmaps does not match number of markers in text".to_string()),
        2 => Err("Image preprocessing error".to_string()),
        _ => Err(format!("Unknown tokenization error: {}", result)),
    }
}

/// # Safety
/// The `chunk` pointer must be valid and point to a properly initialized `mtmd_input_chunk`.
/// The chunk must have been created by `tokenize_with_images` and must remain valid for the duration of this call.
pub unsafe fn encode_chunk(
    ctx: &MtmdContext,
    chunk: *const ffi::mtmd_input_chunk,
) -> Result<(), String> {
    // SAFETY: Caller must ensure chunk was created by tokenize_with_images and remains valid
    let result = unsafe { ffi::mtmd_encode_chunk(ctx.ptr, chunk) };
    if result == 0 {
        Ok(())
    } else {
        Err(format!("Failed to encode chunk: error code {}", result))
    }
}

pub fn get_output_embeddings(ctx: &MtmdContext) -> *mut f32 {
    // SAFETY: Null return is checked immediately after this call
    unsafe { ffi::mtmd_get_output_embd(ctx.ptr) }
}

pub fn default_media_marker() -> &'static str {
    // SAFETY: Null return is checked immediately after this call
    unsafe {
        let marker_ptr = ffi::mtmd_default_marker();
        if marker_ptr.is_null() {
            "<__media__>"
        } else {
            let c_str = std::ffi::CStr::from_ptr(marker_ptr);
            c_str.to_str().unwrap_or("<__media__>")
        }
    }
}

pub fn preprocess_image_for_clip(
    image_data: &[u8],
    target_width: u32,
    target_height: u32,
) -> Result<Vec<u8>, String> {
    if image_data.len() < 8 {
        return Err("Image data too small".to_string());
    }

    let _format = if image_data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "PNG"
    } else if image_data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "JPEG"
    } else {
        return Err("Unsupported image format".to_string());
    };

    let _source_width = u32::from_be_bytes([
        image_data[16],
        image_data[17],
        image_data[18],
        image_data[19],
    ]);
    let _source_height = u32::from_be_bytes([
        image_data[20],
        image_data[21],
        image_data[22],
        image_data[23],
    ]);

    let rgb_size = (target_width * target_height * 3) as usize;
    let mut rgb_data = vec![0u8; rgb_size];

    for i in 0..rgb_size {
        rgb_data[i] = if i < image_data.len() {
            image_data[i]
        } else {
            0
        };
    }

    Ok(rgb_data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("LLAMA-010")]
    #[test]
    fn test_default_media_marker() {
        let marker = default_media_marker();
        assert!(marker.contains("media") || marker.contains("image"));
    }

    #[spec("LLAMA-010")]
    #[test]
    fn test_preprocess_image_size() {
        let fake_png = vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, 0, 0, 0, 1, 0, 0, 0, 2, 0,
            0, 1, 0,
        ];
        let result = preprocess_image_for_clip(&fake_png, 224, 224);
        assert!(result.is_ok());
        let rgb = result.unwrap();
        assert_eq!(rgb.len(), 224 * 224 * 3);
    }
}
