use thiserror::Error;

use crate::automaton::error::CompilerError;
use crate::query_rewrite::json_schema_parser;
use rsonpath_syntax::error::ParseError;

/// Errors returned by the query rewrite pipeline.
#[derive(Debug, Error)]
pub enum QueryRewriteError {
    #[error(transparent)]
    QueryParse(#[from] ParseError),
    #[error(transparent)]
    QueryCompilation(#[from] CompilerError),
    #[error("failed to parse JSON schema: {0:?}")]
    SchemaParsing(json_schema_parser::ParsingError),
}

impl From<json_schema_parser::ParsingError> for QueryRewriteError {
    fn from(value: json_schema_parser::ParsingError) -> Self {
        Self::SchemaParsing(value)
    }
}
