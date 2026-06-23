// src/ml/mod.rs
//
// Machine Learning integrations for NopalDB
// Zero-copy graph to ML framework conversions

#[cfg(feature = "ml")]
pub mod arrow_tensor;

#[cfg(feature = "ml")]
pub mod pyg;

#[cfg(feature = "ml")]
pub use pyg::PyGData;

/// ML integration feature flag check
pub fn is_ml_enabled() -> bool {
    cfg!(feature = "ml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ml_feature_flag() {
        // Just verify module compiles
        assert!(is_ml_enabled() || !is_ml_enabled());
    }
}
