use std::collections::HashSet;

use crate::automaton::{error::CompilerError, Automaton};
use crate::query_rewrite::RewriteContext;
use crate::query_rewrite::{
    optimizer::{QueryCandidateGenerator, QueryRewriteError},
    product::has_nonempty_intersection_of_symmetric_difference,
};
use rsonpath_syntax::JsonPathQuery;

const LOG_TARGET: &str = "rsonpath_lib::query_rewrite::optimizer::pipeline";

/// Generator-agnostic query rewrite pipeline.
///
/// The pipeline:
/// 1. starts from the user query,
/// 2. asks all registered generators for candidate rewrites,
/// 3. compiles candidates to automata,
/// 4. compares them against the original query automaton under the document
///    automaton `d`,
/// 5. returns all equivalent candidates.
///
/// If a generated candidate cannot be compiled into an automaton, it is skipped.
/// The original query is always included in the candidate pool, so the result is
/// never empty as long as the input query itself is supported.
pub struct QueryRewritePipeline<'a> {
    generators: Vec<&'a dyn QueryCandidateGenerator>,
    context: &'a RewriteContext,
}

impl<'a> QueryRewritePipeline<'a> {
    #[must_use]
    pub fn new(generators: Vec<&'a dyn QueryCandidateGenerator>, context: &'a RewriteContext) -> Self {
        Self { generators, context }
    }

    pub fn optimize_query(
        &self,
        query: &JsonPathQuery,
    ) -> Result<Vec<JsonPathQuery>, QueryRewriteError> {
        let original_automaton = Automaton::new(query)?;
        let mut seen = HashSet::new();
        let mut equivalent = Vec::new();

        for candidate in self.candidates(query) {
            log::debug!(target: LOG_TARGET, "candidate: {}", candidate.to_string());
            if !seen.insert(candidate.clone()) {
                continue;
            }

            let Ok(candidate_automaton) = compile_candidate(&candidate) else {
                continue;
            };

            let has_symmetric_difference =
                has_nonempty_intersection_of_symmetric_difference(&original_automaton, &candidate_automaton, self.context.automaton());                
            let is_equivalent = !has_symmetric_difference;

            if is_equivalent {
                equivalent.push(candidate);
            }
        }

        equivalent.sort_by_key(|query| query.to_string());
        Ok(equivalent)
    }

    fn candidates(&self, query: &JsonPathQuery) -> Vec<JsonPathQuery> {
        let mut candidates = vec![query.clone()];
        for generator in &self.generators {
            candidates.extend(generator.generate(query));
        }
        candidates
    }
}


fn compile_candidate(query: &JsonPathQuery) -> Result<Automaton, CompilerError> {
    Automaton::new(query)
}
