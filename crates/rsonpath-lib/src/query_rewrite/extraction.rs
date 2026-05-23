//! Document automaton extraction: JSON → NFA → minimal DFA.

use std::{fs, time::Instant};

use serde_json::{from_str, Value};

use crate::{
    automaton::Automaton,
    query_rewrite::{nfa_minimizer::NfaMinimizer, preprocessor::JsonNfaBuilder},
};

const LOG_TARGET: &str = concat!("rsonpath_lib::query_rewrite", "::extraction");

#[derive(Debug, Default)]
struct DocumentExtractor {
    builder: JsonNfaBuilder,
    minimizer: NfaMinimizer,
}

impl DocumentExtractor {
    fn from_value(&self, value: &Value) -> Automaton {
        let t0 = Instant::now();
        log::info!(target: LOG_TARGET, "extraction started");

        let nfa = self.builder.from_value(value);
        let nfa_states = nfa.num_states();
        let result = self.minimizer.minimize(&nfa);

        log::info!(
            target: LOG_TARGET,
            "extraction finished: nfa_states={} -> min_states={}, elapsed={:?}",
            nfa_states,
            result.num_states(),
            t0.elapsed()
        );
        result
    }

    fn from_json(&self, json: &str) -> Automaton {
        let value: Value = from_str(json).expect("Invalid Json Provided");
        self.from_value(&value)
    }

    fn from_file(&self, path: &str) -> Automaton {
        let json = fs::read_to_string(path).expect("Error during file reading");
        self.from_json(&json)
    }
}

#[inline]
#[must_use]
pub fn extract_automaton_from_file(filename: &str) -> Automaton {
    DocumentExtractor::default().from_file(filename)
}

#[inline]
#[must_use]
pub fn extract_automaton_from_json(json: &str) -> Automaton {
    DocumentExtractor::default().from_json(json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automaton::State;

    #[test]
    fn trivial_null() {
        let a = extract_automaton_from_json("null");
        assert!(a.is_select_root_query());
        assert!(a.is_accepting(State::new(1)));
    }

    #[test]
    fn lista_paths_example() {
        let json = r#"{"lista":[{"a":1,"b":2},{"a":2,"c":3}]}"#;
        let a = extract_automaton_from_json(json);
        assert!(a.is_accepting(a.initial_state()));
        assert_eq!(a[a.initial_state()].member_transitions().len(), 1);
    }
}
