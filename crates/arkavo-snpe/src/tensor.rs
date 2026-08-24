use crate::error::{Result, SnpeError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TensorShape {
    pub dims: Vec<usize>,
}

impl TensorShape {
    pub fn new(dims: Vec<usize>) -> Self {
        Self { dims }
    }

    pub fn from_slice(dims: &[usize]) -> Self {
        Self {
            dims: dims.to_vec(),
        }
    }

    pub fn rank(&self) -> usize {
        self.dims.len()
    }

    pub fn total_elements(&self) -> usize {
        self.dims.iter().product()
    }

    pub fn is_compatible(&self, other: &TensorShape) -> bool {
        self.dims == other.dims
    }

    pub fn batch_size(&self) -> Option<usize> {
        self.dims.first().copied()
    }
}

impl From<Vec<usize>> for TensorShape {
    fn from(dims: Vec<usize>) -> Self {
        Self::new(dims)
    }
}

impl std::fmt::Display for TensorShape {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[")?;
        for (i, dim) in self.dims.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", dim)?;
        }
        write!(f, "]")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TensorDataType {
    Float32,
    Float16,
    UInt8,
    Int8,
}

impl TensorDataType {
    pub fn size_in_bytes(&self) -> usize {
        match self {
            TensorDataType::Float32 => 4,
            TensorDataType::Float16 => 2,
            TensorDataType::UInt8 | TensorDataType::Int8 => 1,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            TensorDataType::Float32 => "float32",
            TensorDataType::Float16 => "float16",
            TensorDataType::UInt8 => "uint8",
            TensorDataType::Int8 => "int8",
        }
    }
}

#[derive(Debug)]
pub struct SnpeTensor {
    pub name: String,
    pub shape: TensorShape,
    pub data_type: TensorDataType,
    pub data: Vec<u8>,
}

impl SnpeTensor {
    pub fn new(name: String, shape: TensorShape, data_type: TensorDataType) -> Self {
        let size = shape.total_elements() * data_type.size_in_bytes();
        Self {
            name,
            shape,
            data_type,
            data: vec![0u8; size],
        }
    }

    pub fn from_f32(name: String, shape: TensorShape, data: Vec<f32>) -> Result<Self> {
        if data.len() != shape.total_elements() {
            return Err(SnpeError::TensorShapeMismatch {
                expected: vec![shape.total_elements()],
                actual: vec![data.len()],
            });
        }

        let bytes: Vec<u8> = data.iter().flat_map(|f| f.to_le_bytes()).collect();

        Ok(Self {
            name,
            shape,
            data_type: TensorDataType::Float32,
            data: bytes,
        })
    }

    pub fn to_f32(&self) -> Result<Vec<f32>> {
        if self.data_type != TensorDataType::Float32 {
            return Err(SnpeError::InvalidTensorType(format!(
                "Expected float32, got {}",
                self.data_type.name()
            )));
        }

        let mut result = Vec::with_capacity(self.shape.total_elements());
        for chunk in self.data.as_chunks::<4>().0 {
            result.push(f32::from_le_bytes(*chunk));
        }

        Ok(result)
    }

    pub fn from_u8(name: String, shape: TensorShape, data: Vec<u8>) -> Result<Self> {
        if data.len() != shape.total_elements() {
            return Err(SnpeError::TensorShapeMismatch {
                expected: vec![shape.total_elements()],
                actual: vec![data.len()],
            });
        }

        Ok(Self {
            name,
            shape,
            data_type: TensorDataType::UInt8,
            data,
        })
    }

    pub fn size_in_bytes(&self) -> usize {
        self.data.len()
    }

    pub fn reshape(&mut self, new_shape: TensorShape) -> Result<()> {
        if new_shape.total_elements() != self.shape.total_elements() {
            return Err(SnpeError::TensorShapeMismatch {
                expected: vec![self.shape.total_elements()],
                actual: vec![new_shape.total_elements()],
            });
        }
        self.shape = new_shape;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tensor_shape() {
        let shape = TensorShape::new(vec![1, 3, 224, 224]);
        assert_eq!(shape.rank(), 4);
        assert_eq!(shape.total_elements(), 150528);
        assert_eq!(shape.batch_size(), Some(1));
    }

    #[test]
    fn test_tensor_from_f32() {
        let data = vec![1.0, 2.0, 3.0, 4.0];
        let shape = TensorShape::new(vec![2, 2]);
        let tensor = SnpeTensor::from_f32("test".to_string(), shape, data.clone()).unwrap();

        assert_eq!(tensor.data_type, TensorDataType::Float32);
        assert_eq!(tensor.to_f32().unwrap(), data);
    }

    #[test]
    fn test_tensor_reshape() {
        let data = vec![1.0, 2.0, 3.0, 4.0];
        let shape = TensorShape::new(vec![4]);
        let mut tensor = SnpeTensor::from_f32("test".to_string(), shape, data).unwrap();

        let new_shape = TensorShape::new(vec![2, 2]);
        assert!(tensor.reshape(new_shape).is_ok());
        assert_eq!(tensor.shape.dims, vec![2, 2]);
    }

    #[test]
    fn test_tensor_shape_mismatch() {
        let data = vec![1.0, 2.0, 3.0];
        let shape = TensorShape::new(vec![2, 2]);
        let result = SnpeTensor::from_f32("test".to_string(), shape, data);
        assert!(result.is_err());
    }
}
