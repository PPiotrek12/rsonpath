//! Extract a document [`Automaton`] via [`super::extraction`], subset determinization,
//! and DFA minimization (Hopcroft).
//!
//! Array steps use a single LIST symbol (abstract path segment `[]`); emission uses
//! [`new_array_transition`](super::helpers::new_array_transition) like [`super::extraction`].
//!
//! ## Diagnostics (hang vs slow work)
//!
//! Enable the `log` target for this module, e.g.:
//! `RUST_LOG=rsonpath::query_rewrite::extraction2=debug` (or `=trace` for per-round minimization).
//! Phase timings and periodic counters show whether execution is progressing or stuck.
//!
//! **Determinization** can sit on the **first** BFS step for a long time: one DFA state’s subset may
//! contain millions of NFA ids, and each of the `alphabet_size` symbols scans that whole subset.
//! For `|subset| ≥ 10_000`, this module logs **`warn`** heartbeats every 16 symbols and **`trace`**
//! progress every 500_000 NFA states scanned within a symbol; tune the constants at the top of this file.

use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    fs,
    time::Instant,
};
use crate::debug;

use serde_json::{from_str, Value};
use smallvec::SmallVec;

/// `log` target for opt-in progress / timing output (works in release builds).
const LOG_TARGET: &str = "rsonpath::query_rewrite::extraction2";

/// Log a heartbeat every this many BFS dequeue steps in determinization (large JSON).
const DETERMINIZE_DEQUEUE_EVERY: u64 = 500;

/// Inside one DFA state, log every N alphabet symbols when subset is huge (progress within first dequeue).
const DETERMINIZE_SYMBOL_HEARTBEAT: usize = 16;

/// When `|subset NFA states| >= this`, log scanning progress inside the subset for each symbol.
const DETERMINIZE_LARGE_SUBSET: usize = 10_000;

/// Inside a large subset, log every N NFA states scanned (per symbol).
const DETERMINIZE_Q_SCAN_EVERY: u64 = 500_000;

/// Second phase (fill `trans` table): log every N DFA rows.
const DETERMINIZE_FILL_ROW_EVERY: usize = 50_000;

/// Log Hopcroft progress at `debug` every this many worklist pops.
const HOPCROFT_DEBUG_EVERY: u32 = 100;

use crate::{
    automaton::{ArrayTransition, Automaton, MemberTransition, State, StateAttributes, StateTable},
    query_rewrite::{
        preprocessor::extract_automaton_from_value,
        helpers::{new_array_transition, new_dumpster_state, new_member_transition},
    },
};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
enum Symbol {
    List,
    Member(String),
}

struct Nfa {
    transitions: Vec<Vec<(Symbol, u32)>>,
    accepting: Vec<bool>,
}

impl Nfa {
    fn new() -> Self {
        Self {
            transitions: Vec::new(),
            accepting: Vec::new(),
        }
    }

    fn add_state(&mut self, accepting: bool) -> u32 {
        let id = self.transitions.len() as u32;
        self.transitions.push(Vec::new());
        self.accepting.push(accepting);
        id
    }

    fn add_transition(&mut self, from: u32, sym: Symbol, to: u32) {
        self.transitions[from as usize].push((sym, to));
    }

    fn alphabet(&self) -> Vec<Symbol> {
        let mut set = BTreeSet::new();
        for outgoing in &self.transitions {
            for (sym, _) in outgoing {
                set.insert(sym.clone());
            }
        }
        set.into_iter().collect()
    }
}

/// Convert a document automaton from [`super::extraction`] into the internal NFA representation.
fn automaton_to_nfa(automaton: &Automaton) -> (Nfa, u32) {
    let mut nfa = Nfa::new();
    for i in 0..automaton.num_states() {
        let accepting = automaton.is_accepting(State::new(i as u32));
        nfa.add_state(accepting);
    }

    for from in 0..automaton.num_states() {
        let table = &automaton[State::new(from as u32)];
        for (pattern, target) in table.member_transitions() {
            let label = std::str::from_utf8(pattern.unquoted())
                .expect("member label must be valid UTF-8")
                .to_owned();
            nfa.add_transition(from as u32, Symbol::Member(label), target.id());
        }
        for array_transition in table.array_transitions() {
            nfa.add_transition(from as u32, Symbol::List, array_transition.target_state().id());
        }
    }

    (nfa, automaton.initial_state().id())
}

fn determinize(nfa: &Nfa, alphabet: &[Symbol], initial: u32) -> (Vec<BTreeSet<u32>>, Vec<Vec<usize>>, usize) {
    let t0 = Instant::now();
    log::debug!(
        target: LOG_TARGET,
        "determinize: BFS phase 1 starting — nfa_states={}, alphabet_size={}, wall_clock={:?}",
        nfa.transitions.len(),
        alphabet.len(),
        t0.elapsed()
    );

    let mut set_to_id: HashMap<BTreeSet<u32>, usize> = HashMap::new();
    let mut sets: Vec<BTreeSet<u32>> = Vec::new();
    let mut queue: VecDeque<usize> = VecDeque::new();

    let start: BTreeSet<u32> = [initial].into();
    set_to_id.insert(start.clone(), 0);
    sets.push(start);
    queue.push_back(0);

    let mut dequeue_count: u64 = 0;
    while let Some(sid) = queue.pop_front() {
        dequeue_count += 1;
        if dequeue_count == 1 {
            log::debug!(
                target: LOG_TARGET,
                "determinize: first dequeue — sid={} (processing {} alphabet symbols × NFA subset size); if subset is huge this step takes a long time before the next log",
                sid,
                alphabet.len()
            );
        }
        if dequeue_count % DETERMINIZE_DEQUEUE_EVERY == 0 {
            log::warn!(
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
                && (sym_idx == 0 || sym_idx % DETERMINIZE_SYMBOL_HEARTBEAT == 0 || sym_idx + 1 == alphabet.len())
            {
                log::warn!(
                    target: LOG_TARGET,
                    "determinize: sid={} subset_size={} symbol_idx={}/{} inner_elapsed={:?} total_elapsed={:?}",
                    sid,
                    cur_len,
                    sym_idx,
                    alphabet.len(),
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
                for (s, t) in &nfa.transitions[q as usize] {
                    if s == sym {
                        next.insert(*t);
                    }
                }
            }
            use std::collections::hash_map::Entry;
            match set_to_id.entry(next.clone()) {
                Entry::Occupied(_) => {}
                Entry::Vacant(v) => {
                    let nid = sets.len();
                    v.insert(nid);
                    sets.push(next);
                    queue.push_back(nid);
                }
            }
        }

        log::trace!(
            target: LOG_TARGET,
            "determinize: sid={} subset_size={} finished_all_symbols inner_elapsed={:?}",
            sid,
            cur_len,
            dequeue_started.elapsed()
        );
    }

    log::debug!(
        target: LOG_TARGET,
        "determinize: BFS phase 1 done — dequeues={}, dfa_states={}, elapsed={:?}; starting transition table fill",
        dequeue_count,
        sets.len(),
        t0.elapsed()
    );

    let n = sets.len();
    let sym_count = alphabet.len();
    let mut trans = vec![vec![0usize; sym_count]; n];

    let fill_t0 = Instant::now();
    for sid in 0..n {
        if sid > 0 && sid % DETERMINIZE_FILL_ROW_EVERY == 0 {
            log::warn!(
                target: LOG_TARGET,
                "determinize: filling trans table — row {}/{} elapsed={:?}",
                sid,
                n,
                fill_t0.elapsed()
            );
        }
        let current = &sets[sid];
        let row_len = current.len();
        let row_started = Instant::now();
        for (j, sym) in alphabet.iter().enumerate() {
            if row_len >= DETERMINIZE_LARGE_SUBSET
                && (j == 0 || j % DETERMINIZE_SYMBOL_HEARTBEAT == 0 || j + 1 == alphabet.len())
            {
                log::warn!(
                    target: LOG_TARGET,
                    "determinize: fill sid={} subset_size={} symbol_idx={}/{} fill_phase_elapsed={:?}",
                    sid,
                    row_len,
                    j,
                    alphabet.len(),
                    fill_t0.elapsed()
                );
            }

            let mut next = BTreeSet::new();
            let mut q_idx: u64 = 0;
            for &q in current {
                q_idx += 1;
                if row_len >= DETERMINIZE_LARGE_SUBSET && q_idx % DETERMINIZE_Q_SCAN_EVERY == 0 {
                    log::trace!(
                        target: LOG_TARGET,
                        "determinize: fill sid={} symbol_idx={} scanned_nfa_states={}/{}",
                        sid,
                        j,
                        q_idx,
                        row_len
                    );
                }
                for (s, t) in &nfa.transitions[q as usize] {
                    if s == sym {
                        next.insert(*t);
                    }
                }
            }
            trans[sid][j] = *set_to_id.get(&next).expect("subset must exist");
        }
        log::trace!(
            target: LOG_TARGET,
            "determinize: fill sid={} row_inner_elapsed={:?}",
            sid,
            row_started.elapsed()
        );
    }

    let initial_id = *set_to_id.get(&[initial].into()).expect("initial subset");

    log::debug!(
        target: LOG_TARGET,
        "determinize: finished — dequeues={}, dfa_states={}, alphabet_size={}, total_elapsed={:?} (fill_rows included)",
        dequeue_count,
        sets.len(),
        alphabet.len(),
        t0.elapsed()
    );

    (sets, trans, initial_id)
}

fn dfa_accepting(nfa: &Nfa, sets: &[BTreeSet<u32>]) -> Vec<bool> {
    sets.iter()
        .map(|set| {
            if set.is_empty() {
                false
            } else {
                set.iter().any(|&q| nfa.accepting[q as usize])
            }
        })
        .collect()
}

fn hopcroft_minimize(trans: &[Vec<usize>], accepting: &[bool]) -> Vec<usize> {
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
                "hopcroft_minimize: exceeded sanity limit (pops {} > limit {}); dfa_states={}, alphabet={}",
                pops,
                sanity_pop_limit,
                n,
                m
            );
            panic!(
                "extraction2::hopcroft_minimize: did not stabilize within {} worklist pops (dfa_states={}, alphabet_size={})",
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
            let (mut in_splitter, mut outside): (Vec<usize>, Vec<usize>) = block
                .into_iter()
                .partition(|&s| splitter.contains(&trans[s][sym]));
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
            log::debug!(
                target: LOG_TARGET,
                "hopcroft_minimize: pops={}, blocks={}, elapsed={:?}",
                pops,
                blocks.len(),
                t0.elapsed()
            );
        }
    }

    log::debug!(
        target: LOG_TARGET,
        "hopcroft_minimize: finished — pops={}, final_blocks={}, elapsed={:?}",
        pops,
        blocks.len(),
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

fn debug_log_automaton_edges(automaton: &Automaton) {
    for state in 0..automaton.num_states() {
        let id = State::new(state as u32);
        let table = &automaton[id];
        for (pattern, target) in table.member_transitions() {
            let label = std::str::from_utf8(pattern.unquoted()).unwrap_or("[invalid utf8]");
            log::debug!(
                target: LOG_TARGET,
                "final edge: {} --{}-> {}",
                state,
                label,
                target.id()
            );
        }
        for array_transition in table.array_transitions() {
            log::debug!(
                target: LOG_TARGET,
                "final edge: {} --[]-> {}",
                state,
                array_transition.target_state().id()
            );
        }
        let fallback = table.fallback_state();
        if fallback.id() != 0 {
            log::debug!(
                target: LOG_TARGET,
                "final edge: {} --*-> {}",
                state,
                fallback.id()
            );
        }
    }
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

fn build_minimized_tables(
    trans: &[Vec<usize>],
    accepting: &[bool],
    alphabet: &[Symbol],
    repr: &[usize],
    dead_old: usize,
    initial_old: usize,
) -> Vec<StateTable> {
    let dead_repr = repr[dead_old];
    let init_repr = repr[initial_old];

    let rep_to_out = remap_repr_to_output_ids(repr, dead_repr, init_repr);
    let max_id = rep_to_out.values().copied().max().expect("non-empty");
    let mut tables: Vec<Option<StateTable>> = vec![None; max_id + 1];

    tables[0] = Some(new_dumpster_state());

    let representatives: BTreeSet<usize> = repr.iter().copied().collect();

    for &r in &representatives {
        let cid = *rep_to_out.get(&r).expect("class");
        if cid == 0 {
            continue;
        }

        let attrs = if accepting[r] {
            StateAttributes::ACCEPTING
        } else {
            StateAttributes::REJECTING
        };

        let mut members: SmallVec<[MemberTransition; 2]> = SmallVec::new();
        let mut arrays: SmallVec<[ArrayTransition; 2]> = SmallVec::new();

        for (j, sym) in alphabet.iter().enumerate() {
            let tgt = trans[r][j];
            let tgt_repr = repr[tgt];
            let tgt_out = *rep_to_out.get(&tgt_repr).expect("target class");
            if tgt_out == 0 {
                continue;
            }
            match sym {
                Symbol::List => arrays.push(new_array_transition(State::new(tgt_out as u32))),
                Symbol::Member(name) => members.push(new_member_transition(name, State::new(tgt_out as u32))),
            }
        }

        tables[cid] = Some(StateTable::new(attrs, members, arrays, State::new(0)));
    }

    tables.into_iter().map(|t| t.expect("filled")).collect()
}

fn extract_automaton_impl(value: &Value) -> Automaton {
    let pipeline_t0 = Instant::now();
    log::debug!(target: LOG_TARGET, "extract_automaton_impl: start");

    let t_nfa = Instant::now();
    let document_automaton = extract_automaton_from_value(value);
    let (nfa, root) = automaton_to_nfa(&document_automaton);
    log::debug!(
        target: LOG_TARGET,
        "extract_automaton_impl: NFA from extraction — nfa_states={}, elapsed={:?}",
        nfa.transitions.len(),
        t_nfa.elapsed()
    );

    let alphabet = nfa.alphabet();
    if alphabet.is_empty() {
        log::debug!(
            target: LOG_TARGET,
            "extract_automaton_impl: empty alphabet (scalar root), total_elapsed={:?}",
            pipeline_t0.elapsed()
        );
        let mut states = vec![new_dumpster_state()];
        states.push(StateTable::new(
            StateAttributes::ACCEPTING,
            SmallVec::new(),
            SmallVec::new(),
            State::new(0),
        ));
        return Automaton::from_states(states);
    }

    log::debug!(
        target: LOG_TARGET,
        "extract_automaton_impl: alphabet_size={}",
        alphabet.len()
    );

    let t_det = Instant::now();
    let (sets, trans, initial_id) = determinize(&nfa, &alphabet, root);
    log::debug!(
        target: LOG_TARGET,
        "extract_automaton_impl: determinize wall_clock={:?}",
        t_det.elapsed()
    );

    let accepting = dfa_accepting(&nfa, &sets);

    let dead_id = sets
        .iter()
        .position(|s| s.is_empty())
        .expect("empty subset always generated when alphabet non-empty");

    let t_min = Instant::now();
    let repr = hopcroft_minimize(&trans, &accepting);
    log::debug!(
        target: LOG_TARGET,
        "extract_automaton_impl: hopcroft wall_clock={:?}",
        t_min.elapsed()
    );

    let t_emit = Instant::now();
    let states = build_minimized_tables(&trans, &accepting, &alphabet, &repr, dead_id, initial_id);
    log::debug!(
        target: LOG_TARGET,
        "extract_automaton_impl: emit wall_clock={:?}, output_states={}",
        t_emit.elapsed(),
        states.len()
    );

    let result = Automaton::from_states(states);
    debug_log_automaton_edges(&result);

    log::debug!(
        target: LOG_TARGET,
        "extract_automaton_impl: done — total_elapsed={:?}",
        pipeline_t0.elapsed()
    );
    result
}

#[inline]
#[must_use]
pub fn extract_automaton_from_file(filename: &str) -> Automaton {
    debug!("Starting extraction from file: {}", filename);
    let json = fs::read_to_string(filename).expect("Error during file reading");
    extract_automaton_from_json(&json)
}

#[inline]
#[must_use]
pub fn extract_automaton_from_json(json: &str) -> Automaton {
    let value: Value = from_str(json).expect("Invalid Json Provided");
    extract_automaton_impl(&value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trivial_null() {
        let a = extract_automaton_from_json("null");
        assert!(a.is_select_root_query());
        assert!(a.is_accepting(crate::automaton::State::new(1)));
    }

    #[test]
    fn lista_paths_example() {
        let json = r#"{"lista":[{"a":1,"b":2},{"a":2,"c":3}]}"#;
        let a = extract_automaton_from_json(json);
        assert!(a.is_accepting(a.initial_state()));
        assert_eq!(a[a.initial_state()].member_transitions().len(), 1);
    }
}
