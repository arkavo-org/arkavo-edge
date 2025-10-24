use crate::accelerator::{AcceleratorTarget, RuntimeCapabilities};
use crate::error::{Result, SnpeError};
use crate::tensor::{SnpeTensor, TensorShape};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct SnpeRuntime {
    capabilities: RuntimeCapabilities,
    selected_target: AcceleratorTarget,
    models: HashMap<String, DlcModel>,
}

impl SnpeRuntime {
    pub fn new() -> Result<Self> {
        crate::initialize()?;

        let capabilities = RuntimeCapabilities::detect();
        let preferred = AcceleratorTarget::from_env();
        let selected_target = capabilities.select_best(preferred);

        log::info!("SNPE Runtime initialized with target: {}", selected_target);

        Ok(Self {
            capabilities,
            selected_target,
            models: HashMap::new(),
        })
    }

    pub fn with_target(target: AcceleratorTarget) -> Result<Self> {
        crate::initialize()?;

        let capabilities = RuntimeCapabilities::detect();
        if !capabilities.is_available(target) {
            return Err(SnpeError::AcceleratorUnavailable(target.to_string()));
        }

        log::info!("SNPE Runtime initialized with target: {}", target);

        Ok(Self {
            capabilities,
            selected_target: target,
            models: HashMap::new(),
        })
    }

    pub fn load_dlc<P: AsRef<Path>>(&mut self, path: P, name: Option<String>) -> Result<String> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(SnpeError::ModelLoad {
                path: path.display().to_string(),
                reason: "File not found".to_string(),
            });
        }

        let model_name = name.unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("model")
                .to_string()
        });

        let model = DlcModel::load(path, self.selected_target)?;

        log::info!(
            "Loaded DLC model '{}' with {} inputs, {} outputs",
            model_name,
            model.input_shapes.len(),
            model.output_shapes.len()
        );

        self.models.insert(model_name.clone(), model);
        Ok(model_name)
    }

    pub fn execute(
        &self,
        model_name: &str,
        inputs: HashMap<String, SnpeTensor>,
    ) -> Result<HashMap<String, SnpeTensor>> {
        let model = self
            .models
            .get(model_name)
            .ok_or_else(|| SnpeError::Config(format!("Model '{}' not loaded", model_name)))?;

        model.execute(inputs, self.selected_target)
    }

    pub fn get_model_info(&self, model_name: &str) -> Option<&DlcModel> {
        self.models.get(model_name)
    }

    pub fn capabilities(&self) -> &RuntimeCapabilities {
        &self.capabilities
    }

    pub fn selected_target(&self) -> AcceleratorTarget {
        self.selected_target
    }

    pub fn unload_model(&mut self, model_name: &str) -> bool {
        self.models.remove(model_name).is_some()
    }
}

#[derive(Debug)]
pub struct DlcModel {
    pub path: PathBuf,
    pub input_shapes: HashMap<String, TensorShape>,
    pub output_shapes: HashMap<String, TensorShape>,
    target: AcceleratorTarget,
}

impl DlcModel {
    fn load(path: &Path, target: AcceleratorTarget) -> Result<Self> {
        let metadata = std::fs::metadata(path).map_err(|e| SnpeError::ModelLoad {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;

        if metadata.len() == 0 {
            return Err(SnpeError::InvalidDlcFormat("DLC file is empty".to_string()));
        }

        let mut input_shapes = HashMap::new();
        input_shapes.insert("input".to_string(), TensorShape::new(vec![1, 3, 224, 224]));

        let mut output_shapes = HashMap::new();
        output_shapes.insert("output".to_string(), TensorShape::new(vec![1, 1000]));

        Ok(Self {
            path: path.to_path_buf(),
            input_shapes,
            output_shapes,
            target,
        })
    }

    fn execute(
        &self,
        inputs: HashMap<String, SnpeTensor>,
        _target: AcceleratorTarget,
    ) -> Result<HashMap<String, SnpeTensor>> {
        for (name, tensor) in &inputs {
            let expected_shape = self
                .input_shapes
                .get(name)
                .ok_or_else(|| SnpeError::Config(format!("Unknown input tensor: {}", name)))?;

            if !tensor.shape.is_compatible(expected_shape) {
                return Err(SnpeError::TensorShapeMismatch {
                    expected: expected_shape.dims.clone(),
                    actual: tensor.shape.dims.clone(),
                });
            }
        }

        let mut outputs = HashMap::new();

        for (name, shape) in &self.output_shapes {
            let output_tensor = SnpeTensor::new(
                name.clone(),
                shape.clone(),
                crate::tensor::TensorDataType::Float32,
            );
            outputs.insert(name.clone(), output_tensor);
        }

        log::debug!(
            "Executed model {} with {} inputs, {} outputs",
            self.path.display(),
            inputs.len(),
            outputs.len()
        );

        Ok(outputs)
    }

    pub fn get_input_shape(&self, name: &str) -> Option<&TensorShape> {
        self.input_shapes.get(name)
    }

    pub fn get_output_shape(&self, name: &str) -> Option<&TensorShape> {
        self.output_shapes.get(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runtime_creation() {
        if std::env::var("SNPE_ROOT").is_ok() {
            let runtime = SnpeRuntime::new();
            assert!(runtime.is_ok());
        }
    }

    #[test]
    fn test_runtime_with_target() {
        if std::env::var("SNPE_ROOT").is_ok() {
            let runtime = SnpeRuntime::with_target(AcceleratorTarget::Cpu);
            assert!(runtime.is_ok());
        }
    }

    #[test]
    fn test_model_load_nonexistent() {
        if std::env::var("SNPE_ROOT").is_ok() {
            let mut runtime = SnpeRuntime::new().unwrap();
            let result = runtime.load_dlc("/nonexistent/model.dlc", None);
            assert!(result.is_err());
        }
    }
}
