pub mod json_schema_parser;
pub mod optimizer;
pub mod product;

pub use optimizer::{
    optimize_query, optimize_query_with_generators, optimize_query_with_schema,
    optimize_query_with_schema_file, PrefixToDescendantGenerator, QueryCandidateGenerator, QueryRewriteError,
    QueryRewritePipeline,
};
pub use product::has_nonempty_intersection_of_symmetric_difference;
