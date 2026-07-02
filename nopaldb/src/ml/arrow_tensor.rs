// src/ml/arrow_tensor.rs
//
// Zero-copy conversion from Apache Arrow to ML tensors
// Uses DLPack protocol for interoperability

use arrow::array::*;
use arrow::record_batch::RecordBatch;
use crate::error::{NopalError, Result};

/// Represents a tensor compatible with PyTorch/NumPy
#[derive(Debug, Clone)]
pub struct MLTensor {
    pub shape: Vec<usize>,
    pub dtype: TensorDType,
    pub data: Vec<u8>,  // Raw bytes (little-endian)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TensorDType {
    Float32,
    Float64,
    Int32,
    Int64,
    Bool,
}

impl MLTensor {
    /// Create tensor from Arrow array.
    /// Para arrays sin nulls usa bytemuck::cast_slice (1 copia por columna en lugar de N).
    pub fn from_arrow_array(array: &dyn Array) -> Result<Self> {
        match array.data_type() {
            arrow::datatypes::DataType::Float32 => {
                let float_array = array
                    .as_any()
                    .downcast_ref::<Float32Array>()
                    .ok_or_else(|| NopalError::custom("Type mismatch: expected Float32Array"))?;
                Self::from_float32_array(float_array)
            }
            arrow::datatypes::DataType::Float64 => {
                let float_array = array
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .ok_or_else(|| NopalError::custom("Type mismatch: expected Float64Array"))?;
                Self::from_float64_array(float_array)
            }
            arrow::datatypes::DataType::Int32 => {
                let int_array = array
                    .as_any()
                    .downcast_ref::<Int32Array>()
                    .ok_or_else(|| NopalError::custom("Type mismatch: expected Int32Array"))?;
                Self::from_int32_array(int_array)
            }
            arrow::datatypes::DataType::Int64 => {
                let int_array = array
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .ok_or_else(|| NopalError::custom("Type mismatch: expected Int64Array"))?;
                Self::from_int64_array(int_array)
            }
            _ => Err(NopalError::Custom(format!(
                "Unsupported Arrow type for tensor: {:?}",
                array.data_type()
            ))),
        }
    }

    fn from_float32_array(array: &Float32Array) -> Result<Self> {
        let data = if array.null_count() == 0 {
            // Ruta rápida: sin nulls — bytemuck view + 1 copia al Vec<u8>
            let slice: &[f32] = array.values().as_ref();
            bytemuck::cast_slice(slice).to_vec()
        } else {
            // Fallback: nulls se reemplazan por 0.0
            let mut buf = Vec::with_capacity(array.len() * 4);
            for i in 0..array.len() {
                let v = if array.is_null(i) { 0.0f32 } else { array.value(i) };
                buf.extend_from_slice(&v.to_le_bytes());
            }
            buf
        };

        Ok(MLTensor {
            shape: vec![array.len()],
            dtype: TensorDType::Float32,
            data,
        })
    }

    fn from_float64_array(array: &Float64Array) -> Result<Self> {
        let data = if array.null_count() == 0 {
            let slice: &[f64] = array.values().as_ref();
            bytemuck::cast_slice(slice).to_vec()
        } else {
            let mut buf = Vec::with_capacity(array.len() * 8);
            for i in 0..array.len() {
                let v = if array.is_null(i) { 0.0f64 } else { array.value(i) };
                buf.extend_from_slice(&v.to_le_bytes());
            }
            buf
        };

        Ok(MLTensor {
            shape: vec![array.len()],
            dtype: TensorDType::Float64,
            data,
        })
    }

    fn from_int32_array(array: &Int32Array) -> Result<Self> {
        let data = if array.null_count() == 0 {
            let slice: &[i32] = array.values().as_ref();
            bytemuck::cast_slice(slice).to_vec()
        } else {
            let mut buf = Vec::with_capacity(array.len() * 4);
            for i in 0..array.len() {
                let v = if array.is_null(i) { 0i32 } else { array.value(i) };
                buf.extend_from_slice(&v.to_le_bytes());
            }
            buf
        };

        Ok(MLTensor {
            shape: vec![array.len()],
            dtype: TensorDType::Int32,
            data,
        })
    }

    fn from_int64_array(array: &Int64Array) -> Result<Self> {
        let data = if array.null_count() == 0 {
            let slice: &[i64] = array.values().as_ref();
            bytemuck::cast_slice(slice).to_vec()
        } else {
            let mut buf = Vec::with_capacity(array.len() * 8);
            for i in 0..array.len() {
                let v = if array.is_null(i) { 0i64 } else { array.value(i) };
                buf.extend_from_slice(&v.to_le_bytes());
            }
            buf
        };

        Ok(MLTensor {
            shape: vec![array.len()],
            dtype: TensorDType::Int64,
            data,
        })
    }

    /// Convert Arrow RecordBatch to list of tensors (one per numeric column)
    pub fn from_record_batch(batch: &RecordBatch) -> Result<Vec<Self>> {
        let mut tensors = Vec::new();

        for column in batch.columns() {
            let tensor = Self::from_arrow_array(column.as_ref())?;
            tensors.push(tensor);
        }

        Ok(tensors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::Float32Array;

    #[test]
    fn test_float32_conversion() {
        let array = Float32Array::from(vec![1.0, 2.0, 3.0]);
        let tensor = MLTensor::from_arrow_array(&array).unwrap();

        assert_eq!(tensor.shape, vec![3]);
        assert_eq!(tensor.dtype, TensorDType::Float32);
        assert_eq!(tensor.data.len(), 12); // 3 * 4 bytes
    }

    #[test]
    fn test_int64_conversion() {
        let array = Int64Array::from(vec![10, 20, 30]);
        let tensor = MLTensor::from_arrow_array(&array).unwrap();

        assert_eq!(tensor.shape, vec![3]);
        assert_eq!(tensor.dtype, TensorDType::Int64);
    }

    #[test]
    fn test_from_float32_no_nulls_single_copy() {
        let values = vec![1.0f32, 2.5, 3.14, -1.0];
        let array = Float32Array::from(values.clone());
        let tensor = MLTensor::from_arrow_array(&array).unwrap();

        assert_eq!(tensor.shape, vec![4]);
        assert_eq!(tensor.data.len(), 4 * 4, "Should be 4 floats * 4 bytes each");

        // Verificar contenido byte a byte
        for (i, &v) in values.iter().enumerate() {
            let expected = v.to_le_bytes();
            let actual = &tensor.data[i * 4..(i + 1) * 4];
            assert_eq!(actual, expected, "Mismatch at index {}", i);
        }
    }

    #[test]
    fn test_from_float32_with_nulls() {
        // Arrow nullable array: null en posición 1
        let array = Float32Array::from(vec![Some(1.0f32), None, Some(3.0)]);
        let tensor = MLTensor::from_arrow_array(&array).unwrap();

        assert_eq!(tensor.shape, vec![3]);
        assert_eq!(tensor.data.len(), 12);

        // El null debe ser 0.0
        let null_bytes = &tensor.data[4..8];
        assert_eq!(null_bytes, 0.0f32.to_le_bytes());
    }
}
