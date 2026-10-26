use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Instant,
};

use serde_json::{from_str, Value};
use thiserror::Error;

use crate::{
    automaton::Automaton,
    query_rewrite::{
        json_schema_parser::{self, ParsingError},
        nfa_minimizer::NfaMinimizer,
        preprocessor::JsonNfaBuilder,
    },
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ContextSource {
    SchemaFile(PathBuf),
    DocumentFile(PathBuf),
}

impl ContextSource {
    #[inline]
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::SchemaFile(path) | Self::DocumentFile(path) => path.as_path(),
        }
    }
}

#[derive(Clone)]
pub struct RewriteContext {
    automaton: Automaton,
    source: ContextSource,
}

#[derive(Default)]
pub struct ContextCacheStore {
    cache: HashMap<ContextSource, RewriteContext>,
}

impl ContextCacheStore {
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self { cache: HashMap::new() }
    }

    #[inline]
    #[must_use]
    pub fn get(&self, source: &ContextSource) -> Option<&RewriteContext> {
        self.cache.get(source)
    }

    #[inline]
    pub fn build_and_insert(&mut self, source: ContextSource) -> Result<&RewriteContext, ContextError> {
        let context = RewriteContext::build(source.clone())?;
        self.cache.insert(source.clone(), context);
        Ok(&self.cache[&source])
    }
}

#[derive(Debug, Error)]
pub enum ContextError {
    #[error("failed to read context file `{path}`")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("`{path}` is not valid JSON")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("`{path}` is not a valid JSON schema: {error:?}")]
    Schema { path: String, error: ParsingError },
}

impl RewriteContext {
    #[inline]
    pub fn build(source: ContextSource) -> Result<Self, ContextError> {
        let file_contents = std::fs::read_to_string(source.path()).map_err(|e| ContextError::Read {
            path: source.path().display().to_string(),
            source: e,
        })?;
        let value: Value = from_str(&file_contents).map_err(|e| ContextError::Json {
            path: source.path().display().to_string(),
            source: e,
        })?;

        drop(file_contents);

        let automaton = match &source {
            ContextSource::SchemaFile(_) => {
                json_schema_parser::parse_json_schema(&value).map_err(|e| ContextError::Schema {
                    path: source.path().display().to_string(),
                    error: e,
                })?
            }
            ContextSource::DocumentFile(_) => build_context_from_value(&value),
        };

        Ok(Self { automaton, source })
    }

    #[inline]
    #[must_use]
    pub fn automaton(&self) -> &Automaton {
        &self.automaton
    }

    #[inline]
    #[must_use]
    pub fn source(&self) -> &ContextSource {
        &self.source
    }
}

const LOG_TARGET: &str = concat!("rsonpath_lib::query_rewrite", "::context");

fn build_context_from_value(value: &Value) -> Automaton {
    let t0 = Instant::now();
    log::info!(target: LOG_TARGET, "building context started");

    let nfa = JsonNfaBuilder::default().from_value(value);
    let nfa_states = nfa.num_states();
    let result = NfaMinimizer::default().minimize(&nfa);

    log::info!(
        target: LOG_TARGET,
        "building context finished: nfa_states={} -> min_states={}, elapsed={:?}",
        nfa_states,
        result.num_states(),
        t0.elapsed()
    );
    result
}
