use color_eyre::eyre::Result;
use std::fs;

const paths: &[&str] = &[
    "crates/rsonpath-benchmarks/data/pison/bestbuy_large_record.json",
    "crates/rsonpath-benchmarks/data/pison/nspl_large_record.json",
    "crates/rsonpath-benchmarks/data/pison/twitter_large_record.json",
    "crates/rsonpath-benchmarks/data/pison/walmart_large_record.json",
    "crates/rsonpath-benchmarks/data/pison/wiki_large_record.json",
];

use std::time::Instant;
use serde_json::Value;
use rsonpath_lib::query_rewrite::{
    nfa_minimizer::NfaMinimizer,
    preprocessor::{JsonNfaBuilder, JsonNfaBuilderConfig, JsonNfaBuildMetrics},
};

#[derive(Debug)]
struct RunStats {
    nfa_states: usize,
    min_dfa_states: usize,
    preprocessing_ms: f64,
    hopcroft_ms: f64,
    json_node_count: usize,
}

fn run(value: &Value, hash_cons: bool) -> RunStats {
    let builder = JsonNfaBuilder::new(JsonNfaBuilderConfig { hash_cons });

    let t0 = Instant::now();
    let (nfa, JsonNfaBuildMetrics { json_node_count, nfa_state_count }) = builder.build(value);
    let preprocessing_ms = t0.elapsed().as_secs_f64() * 1_000.0;

    let t1 = Instant::now();
    let min_dfa = NfaMinimizer::default().minimize(&nfa);
    let hopcroft_ms = t1.elapsed().as_secs_f64() * 1_000.0;

    RunStats {
        nfa_states: nfa_state_count,
        min_dfa_states: min_dfa.num_states(),
        preprocessing_ms,
        hopcroft_ms,
        json_node_count,
    }
}


fn main() -> Result<()> {
    for path in paths {
        let json = fs::read_to_string(path)?;
        let value: Value = serde_json::from_str(&json)?;
        
        drop(json);

        println!("Parsed this, {:?}", path);

        let with_cse = run(&value, true);

        println!("{}", path);
        println!("With CSE: {:?}", with_cse);
        println!();
    }
    Ok(())
}
