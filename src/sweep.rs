use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context as _, Result, anyhow, bail};
use toml::Value;

use crate::params::Params;
use crate::summary::RunSummary;

/// One named run definition: its optional label plus its runtime params.
#[derive(Debug, Clone)]
pub struct RunConfig {
    pub name: Option<String>,
    pub params: Params,
}

/// An ordered list of runs parsed from a TOML `[[runs]]` document.
#[derive(Debug, Clone)]
pub struct RunSuite {
    pub runs: Vec<RunConfig>,
}

/// A [`RunSummary`] tagged with the run label that produced it.
#[derive(Debug, Clone)]
pub struct NamedRunSummary {
    pub run_name: Option<String>,
    pub summary: RunSummary,
}

impl RunSuite {
    /// Parse a [`RunSuite`] from a TOML document with top-level `[[runs]]`
    /// entries. Each entry becomes a [`RunConfig`]; `name` (if present)
    /// is extracted as the run label and all other flat keys are kept as
    /// the run's [`Params`].
    pub fn from_toml_str(s: &str) -> Result<Self> {
        let root: Value = toml::from_str(s).context("failed to parse TOML")?;
        let runs = root
            .get("runs")
            .ok_or_else(|| anyhow!("TOML is missing a 'runs' array"))?;
        let arr = runs
            .as_array()
            .ok_or_else(|| anyhow!("'runs' must be an array of tables"))?;
        if arr.is_empty() {
            bail!("'runs' is empty — at least one [[runs]] entry is required");
        }

        let mut configs = Vec::with_capacity(arr.len());
        for (i, entry) in arr.iter().enumerate() {
            configs.push(parse_run_entry(i, entry)?);
        }
        Ok(Self { runs: configs })
    }

    /// Parse a [`RunSuite`] from a TOML file.
    pub fn from_toml_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        Self::from_toml_str(&contents)
    }
}

fn parse_run_entry(index: usize, entry: &Value) -> Result<RunConfig> {
    let table = entry
        .as_table()
        .ok_or_else(|| anyhow!("runs[{index}] must be a table"))?;

    let mut name: Option<String> = None;
    let mut inner: HashMap<String, Value> = HashMap::with_capacity(table.len());

    for (k, v) in table.iter() {
        if k == "name" {
            let s = v
                .as_str()
                .ok_or_else(|| anyhow!("runs[{index}].name must be a string"))?;
            name = Some(s.to_string());
            continue;
        }
        if v.is_table() {
            bail!("runs[{index}].{k} is a nested table; only flat keys are supported");
        }
        inner.insert(k.clone(), v.clone());
    }

    Ok(RunConfig {
        name,
        params: Params::from_flat_map(inner),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
        [[runs]]
        name = "baseline"
        workers = 4
        mode = "none"

        [[runs]]
        name = "high-concurrency"
        workers = 16
        mode = "none"

        [[runs]]
        workers = 2
        mode = "burst"
    "#;

    #[test]
    fn loads_multiple_runs_in_order() {
        let suite = RunSuite::from_toml_str(SAMPLE).unwrap();
        assert_eq!(suite.runs.len(), 3);
        assert_eq!(suite.runs[0].name.as_deref(), Some("baseline"));
        assert_eq!(suite.runs[1].name.as_deref(), Some("high-concurrency"));
        // Third entry has no name key.
        assert!(suite.runs[2].name.is_none());
    }

    #[test]
    fn run_params_are_parsed_per_entry() {
        let suite = RunSuite::from_toml_str(SAMPLE).unwrap();
        assert_eq!(suite.runs[0].params.get_usize("workers").unwrap(), 4);
        assert_eq!(suite.runs[1].params.get_usize("workers").unwrap(), 16);
        assert_eq!(suite.runs[2].params.get_usize("workers").unwrap(), 2);
        assert_eq!(suite.runs[2].params.get_str("mode").unwrap(), "burst");
    }

    #[test]
    fn missing_runs_errors() {
        let err = RunSuite::from_toml_str("[other]\nx = 1\n").unwrap_err();
        assert!(err.to_string().contains("'runs'"));
    }

    #[test]
    fn non_array_runs_errors() {
        let err = RunSuite::from_toml_str("runs = 42\n").unwrap_err();
        assert!(err.to_string().contains("array"));
    }

    #[test]
    fn empty_runs_errors() {
        let err = RunSuite::from_toml_str("runs = []\n").unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn nested_table_inside_a_run_errors() {
        let src = r#"
            [[runs]]
            name = "x"
            [runs.config]
            workers = 1
        "#;
        let err = RunSuite::from_toml_str(src).unwrap_err();
        assert!(err.to_string().contains("nested table"));
    }

    #[test]
    fn non_string_name_errors() {
        let src = r#"
            [[runs]]
            name = 42
            workers = 1
        "#;
        let err = RunSuite::from_toml_str(src).unwrap_err();
        assert!(err.to_string().contains("name"));
    }

    #[test]
    fn run_without_name_still_parses() {
        let src = r#"
            [[runs]]
            workers = 1
        "#;
        let suite = RunSuite::from_toml_str(src).unwrap();
        assert_eq!(suite.runs.len(), 1);
        assert!(suite.runs[0].name.is_none());
        assert_eq!(suite.runs[0].params.get_usize("workers").unwrap(), 1);
    }
}
