/// Shared `log` target prefix for the query-rewrite pipeline (`-v` / `-vv` in rewrite binaries).
pub const LOG_TARGET: &str = "rsonpath_lib::query_rewrite";

pub mod preprocessor;
pub mod nfa_minimizer;
pub mod benchmark;
pub mod extraction;
pub(crate) mod helpers;
pub mod json_schema_parser;
pub mod optimizer;
pub mod product;

pub use optimizer::{
    optimize_query, optimize_query_with_generators, optimize_query_with_schema, optimize_query_with_schema_file,
    optimize_query_without_schema_file, PrefixToDescendantGenerator, QueryCandidateGenerator, QueryRewriteError,
    QueryRewritePipeline,
};
pub use product::has_nonempty_intersection_of_symmetric_difference;
