use std::collections::HashSet;

use crate::automaton::{Automaton, error::CompilerError};
use crate::query_rewrite::{
    json_schema_parser,
    optimizer::{PrefixToDescendantGenerator, QueryCandidateGenerator, QueryRewriteError},
    product::has_nonempty_intersection_of_symmetric_difference,
};
use rsonpath_syntax::JsonPathQuery;

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
}

impl<'a> QueryRewritePipeline<'a> {
    #[must_use]
    pub fn new(generators: Vec<&'a dyn QueryCandidateGenerator>) -> Self {
        Self { generators }
    }

    pub fn optimize_query(&self, query: &JsonPathQuery, d: &Automaton) -> Result<Vec<JsonPathQuery>, QueryRewriteError> {
        let original_automaton = Automaton::new(query)?;
        let mut seen = HashSet::new();
        let mut equivalent = Vec::new();

        for candidate in self.candidates(query) {
            if !seen.insert(candidate.clone()) {
                continue;
            }

            let Ok(candidate_automaton) = compile_candidate(&candidate) else {
                continue;
            };

            if !has_nonempty_intersection_of_symmetric_difference(&original_automaton, &candidate_automaton, d) {
                equivalent.push(candidate);
            }
        }

        equivalent.sort_by_key(|query| query.to_string());
        Ok(equivalent)
    }

    pub fn optimize_query_string(
        &self,
        query: &str,
        d: &Automaton,
    ) -> Result<Vec<JsonPathQuery>, QueryRewriteError> {
        let parsed = rsonpath_syntax::parse(query)?;
        self.optimize_query(&parsed, d)
    }

    pub fn optimize_query_with_schema(
        &self,
        query: &str,
        schema_json: &str,
    ) -> Result<Vec<JsonPathQuery>, QueryRewriteError> {
        let d = json_schema_parser::from_string(schema_json)?;
        self.optimize_query_string(query, &d)
    }

    pub fn optimize_query_with_schema_file(
        &self,
        query: &str,
        schema_path: &str,
    ) -> Result<Vec<JsonPathQuery>, QueryRewriteError> {
        let d = json_schema_parser::from_file(schema_path)?;
        self.optimize_query_string(query, &d)
    }

    fn candidates(&self, query: &JsonPathQuery) -> Vec<JsonPathQuery> {
        let mut candidates = vec![query.clone()];
        for generator in &self.generators {
            candidates.extend(generator.generate(query));
        }
        candidates
    }
}

/// Optimize a parsed query using the default prefix-to-descendant generator.
pub fn optimize_query(query: &JsonPathQuery, d: &Automaton) -> Result<Vec<JsonPathQuery>, QueryRewriteError> {
    let generator = PrefixToDescendantGenerator;
    QueryRewritePipeline::new(vec![&generator]).optimize_query(query, d)
}

/// Optimize a parsed query using an explicit generator list.
pub fn optimize_query_with_generators(
    query: &JsonPathQuery,
    d: &Automaton,
    generators: &[&dyn QueryCandidateGenerator],
) -> Result<Vec<JsonPathQuery>, QueryRewriteError> {
    QueryRewritePipeline::new(generators.to_vec()).optimize_query(query, d)
}

/// Parse both the input query and the JSON schema, then optimize the query with
/// the default prefix-to-descendant generator.
pub fn optimize_query_with_schema(query: &str, schema_json: &str) -> Result<Vec<JsonPathQuery>, QueryRewriteError> {
    let generator = PrefixToDescendantGenerator;
    QueryRewritePipeline::new(vec![&generator]).optimize_query_with_schema(query, schema_json)
}

/// Parse the input query and schema file, then optimize the query with the
/// default prefix-to-descendant generator.
pub fn optimize_query_with_schema_file(
    query: &str,
    schema_path: &str,
) -> Result<Vec<JsonPathQuery>, QueryRewriteError> {
    let generator = PrefixToDescendantGenerator;
    QueryRewritePipeline::new(vec![&generator]).optimize_query_with_schema_file(query, schema_path)
}

fn compile_candidate(query: &JsonPathQuery) -> Result<Automaton, CompilerError> {
    Automaton::new(query)
}
