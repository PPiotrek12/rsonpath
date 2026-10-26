/// Shared `log` target prefix for the query-rewrite pipeline (`-v` / `-vv` in rewrite binaries).
pub const LOG_TARGET: &str = "rsonpath_lib::query_rewrite";

pub mod benchmark;
pub mod context;
pub(crate) mod helpers;
pub mod json_schema_parser;
pub mod nfa_minimizer;
pub mod optimizer;
pub mod preprocessor;
pub mod product;

pub use context::{ContextCacheStore, ContextSource, RewriteContext};

pub use optimizer::{
    PrefixToDescendantGenerator, QueryCandidateGenerator, QueryRewriteError, QueryRewritePipeline,
};

pub use product::has_nonempty_intersection_of_symmetric_difference;
