use color_eyre::eyre::{Result, WrapErr as _};
use serde::Deserialize;
use std::{fs, num::NonZeroUsize, path::{Path, PathBuf}};

/// A JSON-driven rewrite benchmark suite.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suite {
    pub name: String,
    pub data_dir: PathBuf,
    pub iterations: NonZeroUsize,
    pub warmup: usize,
    pub cases: Vec<SuiteCase>,
}

/// One query/document pair, with optional hand-written rewrites.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuiteCase {
    pub id: String,
    pub dataset: String,
    pub document: String,
    pub query: String,
    /// When absent, the rewrite context is inferred from the document.
    #[serde(default)]
    pub schema: Option<String>,
    #[serde(default)]
    pub manual: Vec<String>,
}

impl Suite {
    /// Load a suite from `path`, naming the file in any parse or I/O error.
    #[inline]
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).wrap_err_with(|| format!("failed to read {}", path.display()))?;
        serde_json::from_str(&text).wrap_err_with(|| format!("failed to parse suite {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::Suite;

    const PAPER_TABLE_4: &str =
        include_str!("../../rsonpath-benchmarks/queries/paper-table-4.json");

    #[test]
    fn paper_table_4_deserializes() {
        let suite: Suite =
            serde_json::from_str(PAPER_TABLE_4).expect("committed paper-table-4.json should deserialize");
        assert_eq!(suite.name, "paper-table-4");
        assert_eq!(suite.cases.len(), 12);
        assert_eq!(suite.cases[0].id, "B1");
        assert_eq!(suite.cases[0].manual.len(), 1);
        assert!(suite.cases.iter().find(|case| case.id == "G1").unwrap().manual.is_empty());
    }
}
