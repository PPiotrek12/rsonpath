use std::{
    collections::HashMap,
    fs,
    hash::{DefaultHasher, Hash, Hasher as _},
};

use crate::query_rewrite::helpers::{new_array_transition, new_member_transition};
use crate::{
    automaton::{ArrayTransition, Automaton, MemberTransition, State, StateAttributes, StateTable},
    query_rewrite::helpers::new_dumpster_state,
};
use serde_json::{from_str, Value};
use smallvec::SmallVec;

/// Can be extended to more complex type if needed
#[derive(PartialEq, PartialOrd, Eq, Ord, Hash, Copy, Clone, Debug)]
struct StateHash {
    hash: u64,
}

struct TreeState {
    transitions: Vec<(String, StateHash)>,
}

impl TreeState {
    fn new() -> Self {
        Self { transitions: vec![] }
    }

    fn organize(&mut self) {
        self.transitions.sort();
        self.transitions.dedup();
    }

    fn add_transition(&mut self, label: String, hash: StateHash) {
        self.transitions.push((label, hash));
    }

    fn hash(&mut self) -> StateHash {
        self.organize();

        let mut hasher = DefaultHasher::new();
        self.transitions.hash(&mut hasher);

        StateHash { hash: hasher.finish() }
    }
}

struct TreeMinimizer {
    states: HashMap<StateHash, TreeState>,
}

impl TreeMinimizer {
    fn new() -> Self {
        let mut leaf = TreeState::new();
        let mut initial_map: HashMap<StateHash, TreeState> = HashMap::new();
        initial_map.insert(leaf.hash(), leaf);

        Self { states: initial_map }
    }

    fn build_state(&mut self, current_json: &Value) -> StateHash {
        let mut state = TreeState::new();

        match current_json {
            Value::Object(values) => {
                for (k, v) in values {
                    let child_hash = self.build_state(v);
                    state.add_transition(k.to_string(), child_hash);
                }
            }
            Value::Array(values) => {
                for v in values {
                    let child_hash = self.build_state(v);
                    state.add_transition(String::new(), child_hash);
                }
            }
            _ => {}
        }

        let state_hash = state.hash();

        let _ = self.states.insert(state_hash, state);

        state_hash
    }

    fn build_automaton(&self, root_hash: StateHash) -> Automaton {
        let mut keys: Vec<StateHash> = self.states.keys().copied().collect();

        keys.sort();

        let root_pos = keys
            .iter()
            .position(|&hash| hash == root_hash)
            .expect("Hash for root has to be in states");
        keys.swap(root_pos, 0);

        let remapped_hashes: HashMap<StateHash, State> = keys
            .iter()
            .enumerate()
            .map(|(i, &hash)| (hash, State::new(1 + i as u8)))
            .collect();

        let mut state_table_vec = Vec::with_capacity(self.states.len() + 1);
        state_table_vec.push(new_dumpster_state());

        for hash in &keys {
            let state = self.states.get(hash).expect("Hash must exist in states");

            let mut member_transitions: SmallVec<[MemberTransition; 2]> = SmallVec::new();
            let mut array_transitions: SmallVec<[ArrayTransition; 2]> = SmallVec::new();

            for (label, destination) in &state.transitions {
                let target = *remapped_hashes
                    .get(destination)
                    .expect("State Hash not found in list, but needed in transition");

                if label.is_empty() {
                    array_transitions.push(new_array_transition(target));
                } else {
                    member_transitions.push(new_member_transition(label, target));
                }
            }

            state_table_vec.push(StateTable::new(
                StateAttributes::ACCEPTING,
                member_transitions,
                array_transitions,
                State::new(0),
            ));
        }

        Automaton::from_states(state_table_vec)
    }
}

#[inline]
#[must_use]
pub fn extract_automaton_from_file(filename: &str) -> Automaton {
    let json = fs::read_to_string(filename).expect("Error during file reading");
    extract_automaton_from_json(&json)
}

#[inline]
#[must_use]
pub fn extract_automaton_from_json(json: &str) -> Automaton {
    let value: Value = from_str(json).expect("Invalid Json Provided");

    let mut minimizer = TreeMinimizer::new();

    let root_hash = minimizer.build_state(&value);

    minimizer.build_automaton(root_hash)
}
