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

const LOG_TARGET: &str = concat!("rsonpath_lib::query_rewrite", "::preprocessor");

/// Counts collected while building a document NFA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsonNfaBuildMetrics {
    pub json_node_count: usize,
    pub nfa_state_count: usize,
}

/// Configuration for [`JsonNfaBuilder`].
#[derive(Debug, Clone, Copy)]
pub struct JsonNfaBuilderConfig {
    pub hash_cons: bool,
}

impl Default for JsonNfaBuilderConfig {
    fn default() -> Self {
        Self { hash_cons: true }
    }
}

/// JSON → document NFA ([`Automaton`] before determinization).
#[derive(Debug, Clone)]
pub struct JsonNfaBuilder {
    config: JsonNfaBuilderConfig,
}

impl Default for JsonNfaBuilder {
    fn default() -> Self {
        Self::new(JsonNfaBuilderConfig::default())
    }
}

impl JsonNfaBuilder {
    #[must_use]
    pub const fn new(config: JsonNfaBuilderConfig) -> Self {
        Self { config }
    }

    #[must_use]
    pub const fn config(&self) -> JsonNfaBuilderConfig {
        self.config
    }

    #[must_use]
    pub fn from_value(&self, value: &Value) -> Automaton {
        let (automaton, metrics) = self.build(value);
        log::info!(
            target: LOG_TARGET,
            "Initial automaton built with {} nodes processed to {} states" , metrics.json_node_count, metrics.nfa_state_count,
        );
        automaton
    }

    #[must_use]
    pub fn build(&self, value: &Value) -> (Automaton, JsonNfaBuildMetrics) {
        log::debug!(target: LOG_TARGET, "preprocessing started");
        let mut inner = JsonTreeBuilder::new(self.config);
        let root_hash = inner.build_state(value);
        let metrics = JsonNfaBuildMetrics {
            json_node_count: inner.json_node_count,
            nfa_state_count: inner.states.len(),
        };
        (inner.build_automaton(root_hash), metrics)
    }

    #[must_use]
    pub fn from_json(&self, json: &str) -> Automaton {
        let value: Value = from_str(json).expect("Invalid Json Provided");
        self.from_value(&value)
    }

    #[must_use]
    pub fn from_file(&self, path: &str) -> Automaton {
        let json = fs::read_to_string(path).expect("Error during file reading");
        self.from_json(&json)
    }
}

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

        StateHash {
            hash: hasher.finish(),
        }
    }
}

struct JsonTreeBuilder {
    states: HashMap<StateHash, TreeState>,
    json_node_count: usize,
    config: JsonNfaBuilderConfig,
}

impl JsonTreeBuilder {
    fn new(config: JsonNfaBuilderConfig) -> Self {
        let mut leaf = TreeState::new();
        let mut initial_map: HashMap<StateHash, TreeState> = HashMap::new();
        initial_map.insert(leaf.hash(), leaf);

        Self {
            states: initial_map,
            json_node_count: 0,
            config,
        }
    }

    fn build_state(&mut self, current_json: &Value) -> StateHash {
        self.json_node_count += 1;

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

        let state_hash = if self.config.hash_cons {
            state.hash()
        } else {
            StateHash {
                hash: self.json_node_count as u64,
            }
        };

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
            .map(|(i, &hash)| (hash, State::new(1 + i as u32)))
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
