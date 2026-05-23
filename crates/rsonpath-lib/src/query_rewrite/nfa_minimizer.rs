//! Subset determinization and Hopcroft minimization for document [`Automaton`]s.

use std::{
    collections::{hash_map::Entry, BTreeSet, HashMap, VecDeque},
    time::Instant,
};

use smallvec::SmallVec;

use crate::{
    automaton::{ArrayTransition, Automaton, MemberTransition, State, StateAttributes, StateTable},
    query_rewrite::helpers::{new_array_transition, new_dumpster_state, new_member_transition},
};

const LOG_TARGET: &str = concat!("rsonpath_lib::query_rewrite", "::nfa_minimizer");

const DETERMINIZE_DEQUEUE_EVERY: u64 = 500;
const DETERMINIZE_SYMBOL_HEARTBEAT: usize = 16;
const DETERMINIZE_LARGE_SUBSET: usize = 10_000;
const DETERMINIZE_Q_SCAN_EVERY: u64 = 500_000;
const HOPCROFT_DEBUG_EVERY: u32 = 100;

/// Document alphabet symbol (member label or abstract array step).
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
enum DocumentSymbol {
    List,
    Member(String),
}

/// Dense DFA tables used between determinization and emission / Hopcroft.
struct DfaTables {
    alphabet: Vec<DocumentSymbol>,
    trans: Vec<Vec<usize>>,
    accepting: Vec<bool>,
    initial_id: usize,
    dead_id: usize,
}

/// NFA document automaton → minimal DFA document automaton.
#[derive(Debug, Default)]
pub struct NfaMinimizer;

impl NfaMinimizer {
    /// Run determinization then Hopcroft minimization.
    #[must_use]
    pub fn minimize(&self, nfa: &Automaton) -> Automaton {
        let t0 = Instant::now();
        let alphabet = alphabet_from_automaton(nfa);
        if alphabet.is_empty() {
            return scalar_root_automaton();
        }

        let dfa = determinize(nfa, &alphabet);
        let result = hopcroft_minimize(&dfa);

        log::debug!(
            target: LOG_TARGET,
            "minimize: nfa_states={} -> min_states={}, elapsed={:?}",
            nfa.num_states(),
            result.num_states(),
            t0.elapsed()
        );
        result
    }
}

fn alphabet_from_automaton(automaton: &Automaton) -> Vec<DocumentSymbol> {
    let mut set = BTreeSet::new();
    for i in 0..automaton.num_states() {
        let table = &automaton[State::new(i as u32)];
        for (pattern, _) in table.member_transitions() {
            let label = std::str::from_utf8(pattern.unquoted())
                .expect("member label must be valid UTF-8")
                .to_owned();
            set.insert(DocumentSymbol::Member(label));
        }
        if !table.array_transitions().is_empty() {
            set.insert(DocumentSymbol::List);
        }
    }
    set.into_iter().collect()
}

fn document_successors(automaton: &Automaton, from: State, sym: &DocumentSymbol) -> SmallVec<[State; 4]> {
    let table = &automaton[from];
    let mut out = SmallVec::new();
    match sym {
        DocumentSymbol::List => {
            for array_transition in table.array_transitions() {
                out.push(array_transition.target_state());
            }
        }
        DocumentSymbol::Member(label) => {
            for (pattern, target) in table.member_transitions() {
                let member_label = std::str::from_utf8(pattern.unquoted()).expect("member label must be valid UTF-8");
                if member_label == label {
                    out.push(*target);
                }
            }
        }
    }
    out
}

fn scalar_root_automaton() -> Automaton {
    let mut states = vec![new_dumpster_state()];
    states.push(StateTable::new(
        StateAttributes::ACCEPTING,
        SmallVec::new(),
        SmallVec::new(),
        State::new(0),
    ));
    Automaton::from_states(states)
}

fn subset_determinize(nfa: &Automaton, alphabet: &[DocumentSymbol], initial: u32) -> DfaTables {
    let t0 = Instant::now();
    let sym_count = alphabet.len();
    log::debug!(
        target: LOG_TARGET,
        "determinize: starting — nfa_states={}, alphabet_size={}",
        nfa.num_states(),
        sym_count
    );

    let mut set_to_id: HashMap<BTreeSet<u32>, usize> = HashMap::new();
    let mut sets: Vec<BTreeSet<u32>> = Vec::new();
    let mut trans: Vec<Vec<usize>> = Vec::new();
    let mut queue: VecDeque<usize> = VecDeque::new();

    let start: BTreeSet<u32> = [initial].into();
    set_to_id.insert(start.clone(), 0);
    sets.push(start);
    trans.push(vec![0; sym_count]);
    queue.push_back(0);

    let mut dequeue_count: u64 = 0;
    while let Some(sid) = queue.pop_front() {
        dequeue_count += 1;
        if dequeue_count == 1 {
            log::trace!(
                target: LOG_TARGET,
                "determinize: first dequeue — sid={} alphabet_size={}",
                sid,
                sym_count
            );
        }
        if dequeue_count % DETERMINIZE_DEQUEUE_EVERY == 0 {
            log::trace!(
                target: LOG_TARGET,
                "determinize: BFS — dequeues={}, queue_len={}, dfa_states_so_far={}, elapsed={:?}",
                dequeue_count,
                queue.len(),
                sets.len(),
                t0.elapsed()
            );
        }

        let dequeue_started = Instant::now();
        let current = sets[sid].clone();
        let cur_len = current.len();

        for (sym_idx, sym) in alphabet.iter().enumerate() {
            if cur_len >= DETERMINIZE_LARGE_SUBSET
                && (sym_idx == 0 || sym_idx % DETERMINIZE_SYMBOL_HEARTBEAT == 0 || sym_idx + 1 == sym_count)
            {
                log::trace!(
                    target: LOG_TARGET,
                    "determinize: sid={} subset_size={} symbol_idx={}/{} inner_elapsed={:?} total_elapsed={:?}",
                    sid,
                    cur_len,
                    sym_idx,
                    sym_count,
                    dequeue_started.elapsed(),
                    t0.elapsed()
                );
            }

            let mut next = BTreeSet::new();
            let mut q_idx: u64 = 0;
            for &q in &current {
                q_idx += 1;
                if cur_len >= DETERMINIZE_LARGE_SUBSET && q_idx % DETERMINIZE_Q_SCAN_EVERY == 0 {
                    log::trace!(
                        target: LOG_TARGET,
                        "determinize: sid={} symbol_idx={} scanned_nfa_states={}/{} inner_elapsed={:?}",
                        sid,
                        sym_idx,
                        q_idx,
                        cur_len,
                        dequeue_started.elapsed()
                    );
                }
                for target in document_successors(nfa, State::new(q), sym) {
                    next.insert(target.id());
                }
            }

            let nid = match set_to_id.entry(next) {
                Entry::Occupied(e) => *e.get(),
                Entry::Vacant(v) => {
                    let nid = sets.len();
                    sets.push(v.key().clone());
                    trans.push(vec![0; sym_count]);
                    queue.push_back(nid);
                    v.insert(nid);
                    nid
                }
            };
            trans[sid][sym_idx] = nid;
        }

        log::trace!(
            target: LOG_TARGET,
            "determinize: sid={} subset_size={} finished_all_symbols inner_elapsed={:?}",
            sid,
            cur_len,
            dequeue_started.elapsed()
        );
    }

    let accepting = dfa_accepting(nfa, &sets);
    let dead_id = sets
        .iter()
        .position(|s| s.is_empty())
        .expect("empty subset always generated when alphabet non-empty");
    let initial_id = *set_to_id.get(&[initial].into()).expect("initial subset");

    log::debug!(
        target: LOG_TARGET,
        "determinize: dfa_states={}, alphabet_size={}, elapsed={:?}",
        sets.len(),
        sym_count,
        t0.elapsed()
    );

    DfaTables {
        alphabet: alphabet.to_vec(),
        trans,
        accepting,
        initial_id,
        dead_id,
    }
}

fn dfa_accepting(nfa: &Automaton, sets: &[BTreeSet<u32>]) -> Vec<bool> {
    sets.iter()
        .map(|set| {
            if set.is_empty() {
                false
            } else {
                set.iter().any(|&q| nfa.is_accepting(State::new(q)))
            }
        })
        .collect()
}

fn determinize(nfa: &Automaton, alphabet: &[DocumentSymbol]) -> Automaton {
    let initial = nfa.initial_state().id();
    let tables = subset_determinize(nfa, alphabet, initial);
    let repr: Vec<usize> = (0..tables.trans.len()).collect();
    emit_dfa_automaton(&tables, &repr)
}

fn automaton_to_dfa_tables(automaton: &Automaton) -> DfaTables {
    let alphabet = alphabet_from_automaton(automaton);
    let n = automaton.num_states();
    let m = alphabet.len();
    let mut trans = vec![vec![0; m]; n];
    let mut accepting = vec![false; n];

    for from in 0..n {
        let state = State::new(from as u32);
        accepting[from] = automaton.is_accepting(state);
        for (j, sym) in alphabet.iter().enumerate() {
            let succs = document_successors(automaton, state, sym);
            trans[from][j] = succs.first().map_or(0, |s| s.id() as usize);
        }
    }

    let initial_id = automaton.initial_state().id() as usize;
    let dead_id = 0;

    DfaTables {
        alphabet,
        trans,
        accepting,
        initial_id,
        dead_id,
    }
}

fn hopcroft_minimize_tables(tables: &DfaTables) -> Vec<usize> {
    let trans = &tables.trans;
    let accepting = &tables.accepting;
    let n = trans.len();
    if n == 0 {
        return vec![];
    }
    let m = trans[0].len();
    let t0 = Instant::now();

    let mut blocks: Vec<Vec<usize>> = vec![
        (0..n).filter(|&s| !accepting[s]).collect(),
        (0..n).filter(|&s| accepting[s]).collect(),
    ];
    blocks.retain(|b| !b.is_empty());

    let mut worklist: VecDeque<(BTreeSet<usize>, usize)> = VecDeque::new();
    if blocks.len() >= 2 {
        let smaller: BTreeSet<usize> = if blocks[0].len() <= blocks[1].len() {
            blocks[0].iter().copied().collect()
        } else {
            blocks[1].iter().copied().collect()
        };
        for a in 0..m {
            worklist.push_back((smaller.clone(), a));
        }
    }

    let sanity_pop_limit = n.saturating_mul(m.saturating_add(1)).saturating_add(1000).max(10_000) as u32;
    let mut pops: u32 = 0;

    while let Some((splitter, sym)) = worklist.pop_front() {
        pops += 1;
        if pops > sanity_pop_limit {
            log::error!(
                target: LOG_TARGET,
                "hopcroft: exceeded sanity limit (pops {} > limit {}); dfa_states={}, alphabet={}",
                pops,
                sanity_pop_limit,
                n,
                m
            );
            panic!(
                "nfa_minimizer::hopcroft: did not stabilize within {} worklist pops (dfa_states={}, alphabet_size={})",
                sanity_pop_limit, n, m
            );
        }

        let mut to_split: Vec<usize> = Vec::new();
        for (bi, block) in blocks.iter().enumerate() {
            let has_in = block.iter().any(|&s| splitter.contains(&trans[s][sym]));
            let has_out = block.iter().any(|&s| !splitter.contains(&trans[s][sym]));
            if has_in && has_out {
                to_split.push(bi);
            }
        }

        for bi in to_split.into_iter().rev() {
            let block = blocks.remove(bi);
            let (mut in_splitter, mut outside): (Vec<usize>, Vec<usize>) =
                block.into_iter().partition(|&s| splitter.contains(&trans[s][sym]));
            in_splitter.sort_unstable();
            outside.sort_unstable();

            let in_set: BTreeSet<usize> = in_splitter.iter().copied().collect();
            let out_set: BTreeSet<usize> = outside.iter().copied().collect();
            blocks.push(in_splitter);
            blocks.push(outside);

            let push_set = if in_set.len() <= out_set.len() { in_set } else { out_set };
            for a in 0..m {
                worklist.push_back((push_set.clone(), a));
            }
        }

        if pops == 1 || pops % HOPCROFT_DEBUG_EVERY == 0 {
            log::trace!(
                target: LOG_TARGET,
                "hopcroft: pops={}, blocks={}, elapsed={:?}",
                pops,
                blocks.len(),
                t0.elapsed()
            );
        }
    }

    let final_blocks = blocks.len();
    log::debug!(
        target: LOG_TARGET,
        "hopcroft: output_states={}, elapsed={:?}",
        final_blocks,
        t0.elapsed()
    );

    let mut repr = vec![0usize; n];
    for block in blocks {
        let r = block[0];
        for s in block {
            repr[s] = r;
        }
    }
    repr
}

fn hopcroft_minimize(dfa: &Automaton) -> Automaton {
    let tables = automaton_to_dfa_tables(dfa);
    let repr = hopcroft_minimize_tables(&tables);
    emit_dfa_automaton(&tables, &repr)
}

fn remap_repr_to_output_ids(repr: &[usize], dead_repr: usize, init_repr: usize) -> HashMap<usize, usize> {
    let unique: BTreeSet<usize> = repr.iter().copied().collect();

    let mut map = HashMap::new();
    map.insert(dead_repr, 0);
    map.insert(init_repr, 1);

    let mut next = 2usize;
    for r in unique {
        if map.contains_key(&r) {
            continue;
        }
        map.insert(r, next);
        next += 1;
    }
    map
}

fn emit_dfa_automaton(tables: &DfaTables, repr: &[usize]) -> Automaton {
    let dead_repr = repr[tables.dead_id];
    let init_repr = repr[tables.initial_id];

    let rep_to_out = remap_repr_to_output_ids(repr, dead_repr, init_repr);
    let max_id = rep_to_out.values().copied().max().expect("non-empty");
    let mut state_tables: Vec<Option<StateTable>> = vec![None; max_id + 1];

    state_tables[0] = Some(new_dumpster_state());

    let representatives: BTreeSet<usize> = repr.iter().copied().collect();

    for &r in &representatives {
        let cid = *rep_to_out.get(&r).expect("class");
        if cid == 0 {
            continue;
        }

        let attrs = if tables.accepting[r] {
            StateAttributes::ACCEPTING
        } else {
            StateAttributes::REJECTING
        };

        let mut members: SmallVec<[MemberTransition; 2]> = SmallVec::new();
        let mut arrays: SmallVec<[ArrayTransition; 2]> = SmallVec::new();

        for (j, sym) in tables.alphabet.iter().enumerate() {
            let tgt = tables.trans[r][j];
            let tgt_repr = repr[tgt];
            let tgt_out = *rep_to_out.get(&tgt_repr).expect("target class");
            if tgt_out == 0 {
                continue;
            }
            match sym {
                DocumentSymbol::List => arrays.push(new_array_transition(State::new(tgt_out as u32))),
                DocumentSymbol::Member(name) => {
                    members.push(new_member_transition(name, State::new(tgt_out as u32)));
                }
            }
        }

        state_tables[cid] = Some(StateTable::new(attrs, members, arrays, State::new(0)));
    }

    Automaton::from_states(state_tables.into_iter().map(|t| t.expect("filled")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_rewrite::preprocessor::JsonNfaBuilder;

    #[test]
    fn scalar_empty_alphabet() {
        let nfa = JsonNfaBuilder::default().from_json("null");
        let min = NfaMinimizer::default().minimize(&nfa);
        assert!(min.is_select_root_query());
        assert!(min.is_accepting(State::new(1)));
    }

    #[test]
    fn hopcroft_reduces_or_preserves() {
        let json = r#"{"lista":[{"a":1,"b":2},{"a":2,"c":3}]}"#;
        let nfa = JsonNfaBuilder::default().from_json(json);
        let dfa_only = determinize(&nfa, &alphabet_from_automaton(&nfa));
        let minimized = NfaMinimizer::default().minimize(&nfa);
        assert!(minimized.num_states() <= dfa_only.num_states());
    }

    #[test]
    fn regression_state_counts_match_extraction() {
        use crate::query_rewrite::extraction::extract_automaton_from_json;

        let json = r#"{"lista":[{"a":1,"b":2},{"a":2,"c":3}]}"#;
        let legacy = extract_automaton_from_json(json);
        let nfa = JsonNfaBuilder::default().from_json(json);
        let via_minimizer = NfaMinimizer::default().minimize(&nfa);
        assert_eq!(legacy.num_states(), via_minimizer.num_states());
    }
}
