use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context as _, Result, anyhow};
use toml::Value;

/// Read-only runtime parameters for a single scenario run.
///
/// Loaded from a flat `[params]` TOML table. Values retain their TOML
/// scalar types (integer, float, bool, string) and are extracted via
/// typed getters.
#[derive(Debug, Clone, Default)]
pub struct Params {
    inner: HashMap<String, Value>,
}

impl Params {
    /// An empty parameter set. Used by `Scenario::run()` when no params
    /// are provided.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Parse parameters from a TOML document containing a flat
    /// `[params]` table.
    pub fn from_toml_str(s: &str) -> Result<Self> {
        let root: Value = toml::from_str(s).context("failed to parse TOML")?;
        let params = root
            .get("params")
            .ok_or_else(|| anyhow!("TOML is missing a [params] table"))?;
        let table = params
            .as_table()
            .ok_or_else(|| anyhow!("[params] must be a TOML table"))?;
        let inner = table.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        Ok(Self { inner })
    }

    /// Parse parameters from a TOML file containing a flat `[params]` table.
    pub fn from_toml_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        Self::from_toml_str(&contents)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.inner.contains_key(key)
    }

    pub fn get_i64(&self, key: &str) -> Result<i64> {
        let v = self.require(key)?;
        v.as_integer()
            .ok_or_else(|| anyhow!("param '{key}' is not an integer"))
    }

    pub fn get_u64(&self, key: &str) -> Result<u64> {
        let v = self.get_i64(key)?;
        u64::try_from(v).map_err(|_| anyhow!("param '{key}' cannot be converted to u64"))
    }

    pub fn get_usize(&self, key: &str) -> Result<usize> {
        let v = self.get_i64(key)?;
        usize::try_from(v).map_err(|_| anyhow!("param '{key}' cannot be converted to usize"))
    }

    pub fn get_f64(&self, key: &str) -> Result<f64> {
        let v = self.require(key)?;
        v.as_float()
            .ok_or_else(|| anyhow!("param '{key}' is not a float"))
    }

    pub fn get_bool(&self, key: &str) -> Result<bool> {
        let v = self.require(key)?;
        v.as_bool()
            .ok_or_else(|| anyhow!("param '{key}' is not a bool"))
    }

    pub fn get_str(&self, key: &str) -> Result<&str> {
        let v = self.require(key)?;
        v.as_str()
            .ok_or_else(|| anyhow!("param '{key}' is not a string"))
    }

    fn require(&self, key: &str) -> Result<&Value> {
        self.inner
            .get(key)
            .ok_or_else(|| anyhow!("missing param: {key}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
        [params]
        workers = 4
        duration_ms = 60000
        read_ratio = 80
        ratio = 0.75
        enable_monitor = true
        injection_mode = "none"
    "#;

    #[test]
    fn loads_flat_params() {
        let p = Params::from_toml_str(SAMPLE).unwrap();
        assert!(p.contains("workers"));
        assert!(p.contains("injection_mode"));
        assert!(!p.contains("missing"));
    }

    #[test]
    fn missing_params_table_errors() {
        let err = Params::from_toml_str("[other]\nx = 1\n").unwrap_err();
        assert!(err.to_string().contains("[params]"));
    }

    #[test]
    fn non_table_params_errors() {
        let err = Params::from_toml_str("params = 42\n").unwrap_err();
        assert!(err.to_string().contains("table"));
    }

    #[test]
    fn get_u64_succeeds() {
        let p = Params::from_toml_str(SAMPLE).unwrap();
        assert_eq!(p.get_u64("duration_ms").unwrap(), 60_000);
    }

    #[test]
    fn get_usize_on_positive_integer() {
        let p = Params::from_toml_str(SAMPLE).unwrap();
        assert_eq!(p.get_usize("workers").unwrap(), 4);
    }

    #[test]
    fn get_usize_on_negative_integer_errors() {
        let p = Params::from_toml_str("[params]\nx = -1\n").unwrap();
        let err = p.get_usize("x").unwrap_err();
        assert!(err.to_string().contains("cannot be converted to usize"));
    }

    #[test]
    fn get_bool_succeeds() {
        let p = Params::from_toml_str(SAMPLE).unwrap();
        assert!(p.get_bool("enable_monitor").unwrap());
    }

    #[test]
    fn get_str_succeeds() {
        let p = Params::from_toml_str(SAMPLE).unwrap();
        assert_eq!(p.get_str("injection_mode").unwrap(), "none");
    }

    #[test]
    fn get_f64_succeeds() {
        let p = Params::from_toml_str(SAMPLE).unwrap();
        assert!((p.get_f64("ratio").unwrap() - 0.75).abs() < 1e-9);
    }

    #[test]
    fn wrong_type_errors() {
        let p = Params::from_toml_str(SAMPLE).unwrap();
        assert!(p.get_str("workers").is_err());
        assert!(p.get_bool("workers").is_err());
        assert!(p.get_u64("injection_mode").is_err());
    }

    #[test]
    fn missing_key_errors() {
        let p = Params::from_toml_str(SAMPLE).unwrap();
        let err = p.get_u64("nope").unwrap_err();
        assert!(err.to_string().contains("missing param"));
        assert!(err.to_string().contains("nope"));
    }

    #[test]
    fn nested_table_value_fails_scalar_getter() {
        // Nested tables aren't explicitly rejected at load time; they
        // simply fail the scalar getters with a clear type error.
        let src = r#"
            [params.nested]
            x = 1
        "#;
        let p = Params::from_toml_str(src).unwrap();
        assert!(p.contains("nested"));
        assert!(p.get_u64("nested").is_err());
    }

    #[test]
    fn empty_params_has_no_keys() {
        let p = Params::empty();
        assert!(!p.contains("anything"));
        assert!(p.get_u64("x").is_err());
    }
}
