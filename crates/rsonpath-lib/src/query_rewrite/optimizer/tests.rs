use super::{
    optimize_query_with_generators, optimize_query_with_schema, PrefixToDescendantGenerator, QueryCandidateGenerator,
};
use crate::query_rewrite::json_schema_parser;
use rsonpath_syntax::JsonPathQuery;

#[test]
fn prefix_generator_emits_suffixes_with_descendant_prefix() {
    let query = rsonpath_syntax::parse("$.a.b.c").unwrap();
    let generator = PrefixToDescendantGenerator;

    let generated = generator
        .generate(&query)
        .into_iter()
        .map(|candidate| candidate.to_string())
        .collect::<Vec<_>>();

    assert_eq!(generated, vec!["$..['b']['c']", "$..['c']"]);
}

#[test]
fn optimizer_finds_descendant_title_rewrite_under_schema() {
    let schema = example_schema();

    let optimized = optimize_query_with_schema("$.content[*].title", schema)
        .unwrap()
        .into_iter()
        .map(|query| query.to_string())
        .collect::<Vec<_>>();

    assert_eq!(
        optimized,
        vec!["$..['title']", "$..[*]['title']", "$['content'][*]['title']"]
    );
}

#[test]
fn optimizer_keeps_original_when_descendant_candidate_is_not_equivalent() {
    let query = rsonpath_syntax::parse("$.a").unwrap();
    let d = crate::automaton::Automaton::new(&rsonpath_syntax::parse("$.a").unwrap()).unwrap();
    let fixed = FixedGenerator {
        candidates: vec![rsonpath_syntax::parse("$.b").unwrap()],
    };

    let optimized = optimize_query_with_generators(&query, &d, &[&fixed])
        .unwrap()
        .into_iter()
        .map(|query| query.to_string())
        .collect::<Vec<_>>();

    assert_eq!(optimized, vec!["$['a']"]);
}

#[test]
fn optimizer_is_generator_agnostic() {
    let query = rsonpath_syntax::parse("$.content[*].title").unwrap();
    let d = json_schema_parser::from_string(example_schema()).unwrap();
    let fixed = FixedGenerator {
        candidates: vec![
            rsonpath_syntax::parse("$..title").unwrap(),
            rsonpath_syntax::parse("$.content[*].title").unwrap(),
        ],
    };

    let optimized = optimize_query_with_generators(&query, &d, &[&fixed])
        .unwrap()
        .into_iter()
        .map(|query| query.to_string())
        .collect::<Vec<_>>();

    assert_eq!(optimized, vec!["$..['title']", "$['content'][*]['title']"]);
}

struct FixedGenerator {
    candidates: Vec<JsonPathQuery>,
}

impl QueryCandidateGenerator for FixedGenerator {
    fn generate(&self, _query: &JsonPathQuery) -> Vec<JsonPathQuery> {
        self.candidates.clone()
    }
}

fn example_schema() -> &'static str {
    include_str!("../../../examples/example_schema.json")
}
