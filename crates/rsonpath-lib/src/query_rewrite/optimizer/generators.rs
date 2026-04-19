use rsonpath_syntax::{JsonPathQuery, Segment};

/// Generator of candidate rewrites for a JSONPath query.
///
/// The optimizer itself is generator-agnostic: it takes any number of these
/// generators, asks each of them for candidates, and then keeps only those
/// that are equivalent to the input query in the context of the document
/// automaton.
pub trait QueryCandidateGenerator {
    /// Produce rewrite candidates for `query`.
    fn generate(&self, query: &JsonPathQuery) -> Vec<JsonPathQuery>;
}

/// A simple rewrite generator that removes longer and longer prefixes of the
/// query and replaces the first remaining step with a descendant step.
///
/// For example:
/// - `$.content[*].title` can yield `$..[*].title` and `$..title`
/// - `$.a.b.c` can yield `$..b.c` and `$..c`
#[derive(Debug, Default, Clone, Copy)]
pub struct PrefixToDescendantGenerator;

impl QueryCandidateGenerator for PrefixToDescendantGenerator {
    fn generate(&self, query: &JsonPathQuery) -> Vec<JsonPathQuery> {
        let segments = query.segments();
        let mut result = Vec::with_capacity(segments.len().saturating_sub(1));

        for prefix_len in 1..segments.len() {
            let mut rewritten = segments[prefix_len..].to_vec();
            if let Some(first) = rewritten.first_mut() {
                *first = Segment::Descendant(first.selectors().clone());
                result.push(JsonPathQuery::from_iter(rewritten));
            }
        }

        result
    }
}
