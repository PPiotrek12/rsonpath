mod error;
mod generators;
mod pipeline;

pub use error::QueryRewriteError;
pub use generators::{PrefixToDescendantGenerator, QueryCandidateGenerator};
pub use pipeline::{
    optimize_query, optimize_query_with_generators, optimize_query_with_schema,
    optimize_query_with_schema_file, QueryRewritePipeline,
};

#[cfg(test)]
mod tests;
