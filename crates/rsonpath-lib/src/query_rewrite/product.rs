use crate::{
    automaton::{ArrayTransition, ArrayTransitionLabel, Automaton, State, StateTable},
    StringPattern,
};
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ProductState {
    q1: State,
    q2: State,
    d: State,
}

/// Check whether `(q1 △ q2) ∩ d` accepts any word.
///
/// The intended use of this helper is comparing two JSONPath query automata `q1`
/// and `q2` under some document-language restriction `d`.
///
/// Intuitively:
/// - `q1 △ q2` captures all paths on which the two queries disagree,
/// - intersecting with `d` keeps only paths that are allowed by the document
///   automaton,
/// - so a return value of `true` means that there exists a document/path allowed
///   by `d` on which `q1` and `q2` produce different answers.
///
/// In particular, if this function returns `false`, then `q1` and `q2` are
/// equivalent in the context described by `d`.
///
/// The implementation is intentionally simple: it explores the product automaton
/// on the fly and stops as soon as it reaches a state where:
/// - exactly one of `q1` and `q2` is accepting, and
/// - `d` is also accepting.
///
/// For member transitions it enumerates all explicit labels visible from the
/// current product state, plus one extra fallback case for "any other member".
///
/// For array transitions it scans a finite set of representative indices. The
/// scan goes up to a boundary derived from the currently visible index/slice
/// labels and then through one full common period of all visible slice steps.
#[must_use]
pub fn has_nonempty_intersection_of_symmetric_difference(q1: &Automaton, q2: &Automaton, d: &Automaton) -> bool {
    let initial = ProductState {
        q1: q1.initial_state(),
        q2: q2.initial_state(),
        d: d.initial_state(),
    };

    let mut seen = HashSet::from([initial]);
    let mut worklist = VecDeque::from([initial]);

    while let Some(current) = worklist.pop_front() {
        if is_accepting(q1, q2, d, current) {
            return true;
        }

        for next in successors(q1, q2, d, current) {
            if seen.insert(next) {
                worklist.push_back(next);
            }
        }
    }

    false
}

fn is_accepting(q1: &Automaton, q2: &Automaton, d: &Automaton, state: ProductState) -> bool {
    (q1.is_accepting(state.q1) != q2.is_accepting(state.q2)) && d.is_accepting(state.d)
}

fn successors(q1: &Automaton, q2: &Automaton, d: &Automaton, state: ProductState) -> Vec<ProductState> {
    let mut result = member_successors(q1, q2, d, state);
    result.extend(array_successors(q1, q2, d, state));

    let mut seen = HashSet::new();
    result.retain(|s| seen.insert(*s));
    result
}

fn member_successors(q1: &Automaton, q2: &Automaton, d: &Automaton, state: ProductState) -> Vec<ProductState> {
    let q1_table = &q1[state.q1];
    let q2_table = &q2[state.q2];
    let d_table = &d[state.d];

    let mut labels: Vec<Arc<StringPattern>> = Vec::new();
    collect_member_labels(q1_table, &mut labels);
    collect_member_labels(q2_table, &mut labels);
    collect_member_labels(d_table, &mut labels);

    let mut result = Vec::with_capacity(labels.len() + 1);
    for label in labels {
        result.push(ProductState {
            q1: next_member_state(q1_table, label.as_ref()),
            q2: next_member_state(q2_table, label.as_ref()),
            d: next_member_state(d_table, label.as_ref()),
        });
    }

    // Any other member label triggers the fallback in all three component states.
    result.push(ProductState {
        q1: q1_table.fallback_state(),
        q2: q2_table.fallback_state(),
        d: d_table.fallback_state(),
    });

    result
}

fn collect_member_labels(table: &StateTable, labels: &mut Vec<Arc<StringPattern>>) {
    for (label, _) in table.member_transitions() {
        if !labels.contains(label) {
            labels.push(label.clone());
        }
    }
}

fn next_member_state(table: &StateTable, label: &StringPattern) -> State {
    table
        .member_transitions()
        .iter()
        .find_map(|(candidate, target)| (candidate.as_ref() == label).then_some(*target))
        .unwrap_or_else(|| table.fallback_state())
}

fn array_successors(q1: &Automaton, q2: &Automaton, d: &Automaton, state: ProductState) -> Vec<ProductState> {
    let q1_table = &q1[state.q1];
    let q2_table = &q2[state.q2];
    let d_table = &d[state.d];

    let limit = array_scan_limit([
        q1_table.array_transitions(),
        q2_table.array_transitions(),
        d_table.array_transitions(),
    ]);

    let mut result = Vec::new();
    let mut idx = 0_u64;
    while idx <= limit {
        let index = idx
            .try_into()
            .expect("scan limit is based on existing array labels and must fit JsonUInt");
        result.push(ProductState {
            q1: next_array_state(q1_table, index),
            q2: next_array_state(q2_table, index),
            d: next_array_state(d_table, index),
        });

        if idx == u64::MAX {
            break;
        }
        idx += 1;
    }

    result
}

fn next_array_state(table: &StateTable, index: rsonpath_syntax::num::JsonUInt) -> State {
    table
        .array_transitions()
        .iter()
        .find_map(|transition| transition.matches(index).then_some(transition.target_state()))
        .unwrap_or_else(|| table.fallback_state())
}

fn array_scan_limit<const N: usize>(transition_sets: [&[ArrayTransition]; N]) -> u64 {
    let mut max_boundary = 1_u64;
    let mut period = 1_u64;

    for transitions in transition_sets {
        for transition in transitions {
            update_scan_parameters(transition, &mut max_boundary, &mut period);
        }
    }

    max_boundary.saturating_add(period)
}

fn update_scan_parameters(transition: &ArrayTransition, max_boundary: &mut u64, period: &mut u64) {
    match transition.label() {
        ArrayTransitionLabel::Index(index) => {
            *max_boundary = (*max_boundary).max(index.as_u64());
        }
        ArrayTransitionLabel::Slice(slice) => {
            *max_boundary = (*max_boundary).max(slice.start().as_u64());
            let end = slice.end();
            if let Some(end) = end {
                *max_boundary = (*max_boundary).max(end.as_u64());
            }
            if slice.step().as_u64() > 0 {
                *period = lcm(*period, slice.step().as_u64());
            }
        }
    }
}

fn lcm(a: u64, b: u64) -> u64 {
    if a == 0 || b == 0 {
        0
    } else {
        a / gcd(a, b) * b
    }
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let tmp = a % b;
        a = b;
        b = tmp;
    }
    a
}

#[cfg(test)]
mod tests {
    use super::has_nonempty_intersection_of_symmetric_difference;
    use crate::{
        automaton::{
            ArrayTransition, ArrayTransitionLabel, Automaton, SimpleSlice, State, StateAttributes, StateTable,
        },
        StringPattern,
    };
    use std::sync::Arc;

    #[test]
    fn product_detects_nonempty_member_language() {
        let q1 = Automaton::new(&rsonpath_syntax::parse("$.a").unwrap()).unwrap();
        let q2 = Automaton::new(&rsonpath_syntax::parse("$.b").unwrap()).unwrap();
        let d = Automaton::new(&rsonpath_syntax::parse("$.a").unwrap()).unwrap();

        assert!(has_nonempty_intersection_of_symmetric_difference(&q1, &q2, &d));
    }

    #[test]
    fn product_detects_empty_member_language() {
        let q1 = Automaton::new(&rsonpath_syntax::parse("$.a").unwrap()).unwrap();
        let q2 = Automaton::new(&rsonpath_syntax::parse("$.a").unwrap()).unwrap();
        let d = Automaton::new(&rsonpath_syntax::parse("$.a").unwrap()).unwrap();

        assert!(!has_nonempty_intersection_of_symmetric_difference(&q1, &q2, &d));
    }

    #[test]
    fn product_detects_nonempty_array_language() {
        let q1 = Automaton::new(&rsonpath_syntax::parse("$[2]").unwrap()).unwrap();
        let q2 = Automaton::new(&rsonpath_syntax::parse("$[1:5:2]").unwrap()).unwrap();
        let d = Automaton::new(&rsonpath_syntax::parse("$[3]").unwrap()).unwrap();

        assert!(has_nonempty_intersection_of_symmetric_difference(&q1, &q2, &d));
    }

    #[test]
    fn product_detects_empty_array_language() {
        let q1 = Automaton::new(&rsonpath_syntax::parse("$[2]").unwrap()).unwrap();
        let q2 = Automaton::new(&rsonpath_syntax::parse("$[2]").unwrap()).unwrap();
        let d = Automaton::new(&rsonpath_syntax::parse("$[2]").unwrap()).unwrap();

        assert!(!has_nonempty_intersection_of_symmetric_difference(&q1, &q2, &d));
    }

    #[test]
    fn schema_document_distinguishes_four_step_name_and_descendant_name() {
        let q1 = Automaton::new(&rsonpath_syntax::parse("$.*.*.*.*.name").unwrap()).unwrap();
        let q2 = Automaton::new(&rsonpath_syntax::parse("$..name").unwrap()).unwrap();
        let d = document_schema_automaton();

        assert!(has_nonempty_intersection_of_symmetric_difference(&q1, &q2, &d));
    }

    #[test]
    fn schema_document_equates_content_title_and_descendant_title() {
        let q1 = Automaton::new(&rsonpath_syntax::parse("$.content.*.title").unwrap()).unwrap();
        let q2 = Automaton::new(&rsonpath_syntax::parse("$..title").unwrap()).unwrap();
        let d = document_schema_automaton();

        assert!(!has_nonempty_intersection_of_symmetric_difference(&q1, &q2, &d));
    }

    fn document_schema_automaton() -> Automaton {
        let content_to_item = ArrayTransition::new(
            ArrayTransitionLabel::Slice(SimpleSlice::new(0.into(), None, 1.into())),
            State::new(3),
        );
        let cast_to_person = ArrayTransition::new(
            ArrayTransitionLabel::Slice(SimpleSlice::new(0.into(), None, 1.into())),
            State::new(5),
        );

        Automaton::from_states(vec![
                state_table(vec![], vec![], State::new(0), StateAttributes::REJECTING),
                state_table(
                    vec![member("content", State::new(2))],
                    vec![],
                    State::new(0),
                    StateAttributes::ACCEPTING,
                ),
                state_table(vec![], vec![content_to_item], State::new(0), StateAttributes::ACCEPTING),
                state_table(
                    vec![
                        member("title", State::new(4)),
                        member("author", State::new(5)),
                        member("length", State::new(4)),
                        member("cast", State::new(6)),
                    ],
                    vec![],
                    State::new(0),
                    StateAttributes::ACCEPTING,
                ),
                state_table(vec![], vec![], State::new(0), StateAttributes::ACCEPTING),
                state_table(
                    vec![member("name", State::new(4)), member("age", State::new(4))],
                    vec![],
                    State::new(0),
                    StateAttributes::ACCEPTING,
                ),
                state_table(vec![], vec![cast_to_person], State::new(0), StateAttributes::ACCEPTING),
            ])
    }

    fn state_table(
        member_transitions: Vec<(Arc<StringPattern>, State)>,
        array_transitions: Vec<ArrayTransition>,
        fallback_state: State,
        attributes: StateAttributes,
    ) -> StateTable {
        StateTable::new(
            attributes,
            member_transitions.into_iter().collect(),
            array_transitions.into_iter().collect(),
            fallback_state,
        )
    }

    fn member(label: &str, target: State) -> (Arc<StringPattern>, State) {
        let json_string = rsonpath_syntax::str::JsonString::new(label);
        (Arc::new(StringPattern::new(&json_string)), target)
    }
}
