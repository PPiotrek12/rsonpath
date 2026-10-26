use crate::{
    automaton::error::CompilerError,
    engine::{error::EngineError, Compiler as _, Engine as _, RsonpathEngine},
    input::OwnedBytes,
    result::MatchCount,
};
use rsonpath_syntax::JsonPathQuery;
use std::{num::NonZeroUsize, time::Instant};
use thiserror::Error;

const LOG_TARGET: &str = "rsonpath_lib::query_rewrite::benchmark";

pub struct BenchResult {
    pub query: String,
    pub match_count: usize,
    pub matches_baseline: bool,
    pub compile_ns: u128,
    pub exec_mean_ns: u128,
    pub throughput_bytes_per_s: f64,
}

/// Outcome of measuring a single query. A candidate that fails to compile or run is
/// recorded rather than aborting the whole measurement, so that a generator emitting
/// unsupported queries stays visible in the results instead of killing a suite run.
pub enum Outcome {
    Measured(BenchResult),
    Failed { query: String, error: BenchError },
}

#[derive(Error, Debug)]
pub enum BenchError {
    #[error("failed to compile query {query}: {source}")]
    Compile { query: String, source: CompilerError },
    #[error("failed to execute query {query}: {source}")]
    Execute { query: String, source: EngineError },
    #[error("query {query} returned inconsistent counts: count() = {count}, indices() = {indices}")]
    InconsistentCount {
        query: String,
        count: MatchCount,
        indices: usize,
    },
}

/// Measures every query against the same input. Index 0 is the baseline; the rest are
/// validated by exact match-index equality against it.
///
/// The returned vector is aligned 1:1 with `queries` and in the same order, so callers can
/// map their own labels onto it by index. Index 0 is always [`Outcome::Measured`] — a
/// baseline that fails to compile or run leaves the case with no reference to validate
/// against and no denominator for speedup, so it aborts instead.
#[inline]
pub fn measure(
    document: Vec<u8>,
    queries: &[JsonPathQuery],
    iterations: NonZeroUsize,
    warmup: usize,
) -> Result<Vec<Outcome>, BenchError> {
    let Some((baseline_query, candidates)) = queries.split_first() else {
        return Ok(Vec::new());
    };

    let input_len = document.len();
    let input = OwnedBytes::new(document);

    let (baseline, match_indices) = benchmark_query(baseline_query, &input, iterations, warmup, input_len, None)?;

    let mut outcomes = Vec::with_capacity(queries.len());
    outcomes.push(Outcome::Measured(baseline));

    for query in candidates {
        match benchmark_query(query, &input, iterations, warmup, input_len, Some(&match_indices)) {
            Ok((result, _)) => outcomes.push(Outcome::Measured(result)),
            Err(error) => {
                log::warn!(target: LOG_TARGET, "skipping candidate: {error}");
                outcomes.push(Outcome::Failed {
                    query: query.to_string(),
                    error,
                });
            }
        }
    }

    Ok(outcomes)
}


/// Returns the measurement together with the match indices it produced; the caller keeps
/// the baseline's indices to validate later queries against and drops the rest.
fn benchmark_query(
    query: &JsonPathQuery,
    input: &OwnedBytes<Vec<u8>>,
    iterations: NonZeroUsize,
    warmup: usize,
    document_size_bytes: usize,
    expected_indices: Option<&[usize]>,
) -> Result<(BenchResult, Vec<usize>), BenchError> {
    let query_string = query.to_string();
    let execute_error = |source| BenchError::Execute {
        query: query_string.clone(),
        source,
    };

    let compile_start = Instant::now();
    let engine = RsonpathEngine::compile_query(query).map_err(|source| BenchError::Compile {
        query: query_string.clone(),
        source,
    })?;
    let compile_ns = compile_start.elapsed().as_nanos();

    let mut indices = Vec::new();
    engine.indices(input, &mut indices).map_err(execute_error)?;

    let matches_baseline = expected_indices.is_none_or(|expected| expected == indices.as_slice());

    for _ in 0..warmup {
        engine.count(input).map_err(execute_error)?;
    }

    let mut total_ns = 0_u128;

    for _ in 0..iterations.get() {
        let start = Instant::now();
        let count = engine.count(input).map_err(execute_error)?;
        let elapsed = start.elapsed().as_nanos();

        if usize::try_from(count).ok() != Some(indices.len()) {
            return Err(BenchError::InconsistentCount {
                query: query_string,
                count,
                indices: indices.len(),
            });
        }

        total_ns += elapsed;
    }

    let exec_mean_ns = total_ns / iterations.get() as u128;
    let exec_mean_s = exec_mean_ns as f64 / 1_000_000_000.0;
    let throughput_bytes_per_s = document_size_bytes as f64 / exec_mean_s;

    Ok((
        BenchResult {
            query: query_string,
            match_count: indices.len(),
            matches_baseline,
            compile_ns,
            exec_mean_ns,
            throughput_bytes_per_s,
        },
        indices,
    ))
}
