use thiserror::Error;

use crate::automaton::error::CompilerError;
use rsonpath_syntax::error::ParseError;

/// Errors returned by the query rewrite pipeline.
#[derive(Debug, Error)]
pub enum QueryRewriteError {
    #[error(transparent)]
    QueryParse(#[from] ParseError),
    #[error(transparent)]
    QueryCompilation(#[from] CompilerError),
}

