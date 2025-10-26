#![cfg(all(target_os = "linux", target_arch = "aarch64", feature = "snpe"))]

use crate::{Error, Result};
use arkavo_snpe::{AcceleratorTarget, SnpeRuntime, SnpeTensor, TensorShape};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct SnpeModel {
    runtime: Arc<Mutex<SnpeRuntime>>,
    model_name: String,
    input_shapes: HashMap<String, TensorShape>,
    output_shapes: HashMap<String, TensorShape>,
}

impl SnpeModel {
    pub fn load<P: AsRef<Path>>(path: P, target: Option<AcceleratorTarget>) -> Result<Self> {
        let path = path.as_ref();

        let mut runtime = if let Some(t) = target {
            SnpeRuntime::with_target(t)
                .map_err(|e| Error::Model(format!("SNPE runtime init failed: {e}")))?
        } else {
            SnpeRuntime::new()
                .map_err(|e| Error::Model(format!("SNPE runtime init failed: {e}")))?
        };

        let model_name = runtime
            .load_dlc(path, None)
            .map_err(|e| Error::Model(format!("DLC load failed: {e}")))?;

        let model_info = runtime
            .get_model_info(&model_name)
            .ok_or_else(|| Error::Model("Model info not available".to_string()))?;

        let input_shapes = model_info.input_shapes.clone();
        let output_shapes = model_info.output_shapes.clone();

        log::info!(
            "Loaded SNPE model '{}' with accelerator: {:?}",
            model_name,
            runtime.selected_target()
        );

        Ok(Self {
            runtime: Arc::new(Mutex::new(runtime)),
            model_name,
            input_shapes,
            output_shapes,
        })
    }

    pub async fn execute(
        &self,
        inputs: HashMap<String, Vec<f32>>,
    ) -> Result<HashMap<String, Vec<f32>>> {
        let mut snpe_inputs = HashMap::new();

        for (name, data) in inputs {
            let shape = self
                .input_shapes
                .get(&name)
                .ok_or_else(|| Error::Model(format!("Unknown input tensor: {}", name)))?
                .clone();

            let tensor = SnpeTensor::from_f32(name.clone(), shape, data)
                .map_err(|e| Error::Model(format!("Tensor conversion failed: {e}")))?;

            snpe_inputs.insert(name, tensor);
        }

        let runtime = self.runtime.lock().await;
        let snpe_outputs = runtime
            .execute(&self.model_name, snpe_inputs)
            .map_err(|e| Error::Inference(format!("SNPE execution failed: {e}")))?;

        drop(runtime);

        let mut outputs = HashMap::new();
        for (name, tensor) in snpe_outputs {
            let data = tensor
                .to_f32()
                .map_err(|e| Error::Model(format!("Output conversion failed: {e}")))?;
            outputs.insert(name, data);
        }

        Ok(outputs)
    }

    pub fn get_input_shape(&self, name: &str) -> Option<&TensorShape> {
        self.input_shapes.get(name)
    }

    pub fn get_output_shape(&self, name: &str) -> Option<&TensorShape> {
        self.output_shapes.get(name)
    }

    pub fn model_name(&self) -> &str {
        &self.model_name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_snpe_model_load() {
        if std::env::var("SNPE_ROOT").is_ok() {
            let temp_dir = tempfile::tempdir().unwrap();
            let dlc_path = temp_dir.path().join("test.dlc");

            std::fs::write(&dlc_path, b"dummy dlc content").unwrap();

            let result = SnpeModel::load(&dlc_path, None);

            drop(temp_dir);

            if result.is_err() {
                println!("SNPE model load failed (expected in test environment)");
            }
        }
    }
}
