use crate::{Error, Result};
use candle_core::{Device, Tensor};
use std::path::Path;

pub struct ModelLoader {
    device: Device,
    model_name: String,
    model_path: Option<String>,
}

impl ModelLoader {
    pub fn new(model_name: &str, model_path: Option<&str>) -> Result<Self> {
        // Select device based on platform
        let device = Self::select_device()?;

        Ok(Self {
            device,
            model_name: model_name.to_string(),
            model_path: model_path.map(String::from),
        })
    }

    fn select_device() -> Result<Device> {
        cfg_if::cfg_if! {
            if #[cfg(all(target_os = "macos", target_arch = "aarch64"))] {
                // Try Metal first, fall back to CPU
                match Device::new_metal(0) {
                    Ok(device) => {
                        tracing::info!("Using Metal device for acceleration");
                        Ok(device)
                    }
                    Err(_) => {
                        tracing::warn!("Metal device not available, falling back to CPU");
                        Ok(Device::Cpu)
                    }
                }
            } else {
                // Default to CPU for other platforms
                tracing::info!("Using CPU device");
                Ok(Device::Cpu)
            }
        }
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    pub async fn load_model(&self) -> Result<()> {
        // Placeholder for model loading logic
        // This will be implemented to load GGUF models
        tracing::info!(
            "Loading model '{}' on device {:?}",
            self.model_name,
            self.device
        );

        if let Some(path) = &self.model_path {
            if !Path::new(path).exists() {
                return Err(Error::Config(format!("Model file not found: {}", path)));
            }
        }

        Ok(())
    }

    pub fn create_tensor(&self, data: &[f32], shape: &[usize]) -> Result<Tensor> {
        Tensor::from_slice(data, shape, &self.device)
            .map_err(|e| Error::Model(format!("Failed to create tensor: {}", e)))
    }
}
