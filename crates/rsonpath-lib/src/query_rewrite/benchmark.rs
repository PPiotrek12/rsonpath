use crate::{
    engine::{Compiler as _, Engine as _, RsonpathEngine},
    input::OwnedBytes,
    query_rewrite::{optimize_query_with_schema_file, optimize_query_without_schema_file, QueryRewriteError},
};
use rsonpath_syntax::JsonPathQuery;
use std::{fs, time::Instant};
use thiserror::Error;

const LOG_TARGET: &str = "rsonpath_lib::query_rewrite::benchmark";

#[derive(Debug)]
pub struct RewriteBenchmarkConfig {
    pub query: String,
    pub schema_file: Option<String>,
    pub no_schema: bool,
    pub document_file: String,
    pub iterations: usize,
    pub warmup: usize,
}

#[derive(Debug)]
pub struct FixedQueryBenchmarkConfig {
    pub baseline_query: String,
    pub query: String,
    pub role: &'static str,
    pub document_file: String,
    pub iterations: usize,
    pub warmup: usize,
}

#[derive(Clone, Debug)]
pub struct BenchmarkRow {
    pub query: String,
    pub role: &'static str,
    pub query_len: usize,
    pub match_count: usize,
    pub matches_baseline: bool,
    pub compile_ns: u128,
    pub exec_mean_ns: u128,
    pub throughput_bytes_per_s: f64,
}

#[derive(Debug, Clone)]
pub struct RewriteBenchmarkReport {
    pub rows: Vec<BenchmarkRow>,
    pub generated_candidates: usize,
}

#[derive(Debug, Clone)]
pub struct BestRewriteSelection {
    pub query: String,
    pub row: BenchmarkRow,
    pub report: RewriteBenchmarkReport,
}

struct BenchmarkDetails {
    row: BenchmarkRow,
    match_indices: Vec<usize>,
}

pub fn run_benchmark(config: &RewriteBenchmarkConfig) -> Result<RewriteBenchmarkReport, RewriteBenchmarkError> {
    if config.iterations == 0 {
        return Err(RewriteBenchmarkError::ZeroIterations);
    }

    if !config.no_schema && config.schema_file.is_none() {
        return Err(RewriteBenchmarkError::MissingSchema);
    }

    let baseline_query = rsonpath_syntax::parse(&config.query)?;
    let candidates = if let Some(schema) = &config.schema_file {
        optimize_query_with_schema_file(&config.query, schema)?
    } else {
        optimize_query_without_schema_file(&config.query, &config.document_file)?
    };
    let generated_candidates = candidates.len();
    let document = fs::read(&config.document_file)?;
    let document_size_bytes = document.len();
    let input = OwnedBytes::new(document);

    log::debug!(target: LOG_TARGET, "Benchmarking baseline query: {}", baseline_query.to_string());
    let baseline = benchmark_query(
        &baseline_query,
        "original",
        &input,
        config.iterations,
        config.warmup,
        document_size_bytes,
        None,
    )?;
    let expected_indices = baseline.match_indices.clone();

    let mut rows = Vec::with_capacity(candidates.len() + 1);
    rows.push(baseline.row);

    for candidate in candidates {
        log::debug!(target: LOG_TARGET, "Benchmarking candidate query: {}", candidate.to_string());
        if candidate == baseline_query {
            continue;
        }

        let details = benchmark_query(
            &candidate,
            "rewrite",
            &input,
            config.iterations,
            config.warmup,
            document_size_bytes,
            Some(&expected_indices),
        )?;
        rows.push(details.row);
    }

    let (baseline_row, mut rewrites): (Vec<_>, Vec<_>) = rows.into_iter().partition(|row| row.role == "original");
    rewrites.sort_by(|left, right| right.throughput_bytes_per_s.total_cmp(&left.throughput_bytes_per_s));

    let mut ordered = baseline_row;
    ordered.extend(rewrites);
    Ok(RewriteBenchmarkReport {
        rows: ordered,
        generated_candidates,
    })
}

pub fn best_equivalent_row(report: &RewriteBenchmarkReport) -> Option<&BenchmarkRow> {
    report
        .rows
        .iter()
        .filter(|row| row.matches_baseline)
        .max_by(|left, right| left.throughput_bytes_per_s.total_cmp(&right.throughput_bytes_per_s))
}

pub fn select_best_rewrite(config: &RewriteBenchmarkConfig) -> Result<BestRewriteSelection, RewriteBenchmarkError> {
    let report = run_benchmark(config)?;
    let row = best_equivalent_row(&report)
        .cloned()
        .ok_or(RewriteBenchmarkError::NoEquivalentQuery)?;

    Ok(BestRewriteSelection {
        query: row.query.clone(),
        row,
        report,
    })
}

pub fn benchmark_fixed_query(config: &FixedQueryBenchmarkConfig) -> Result<BenchmarkRow, RewriteBenchmarkError> {
    if config.iterations == 0 {
        return Err(RewriteBenchmarkError::ZeroIterations);
    }

    let baseline_query = rsonpath_syntax::parse(&config.baseline_query)?;
    let query = rsonpath_syntax::parse(&config.query)?;
    let document = fs::read(&config.document_file)?;
    let document_size_bytes = document.len();
    let input = OwnedBytes::new(document);

    let baseline = benchmark_query(
        &baseline_query,
        "baseline",
        &input,
        config.iterations,
        config.warmup,
        document_size_bytes,
        None,
    )?;
    let details = benchmark_query(
        &query,
        config.role,
        &input,
        config.iterations,
        config.warmup,
        document_size_bytes,
        Some(&baseline.match_indices),
    )?;

    Ok(details.row)
}

fn benchmark_query(
    query: &JsonPathQuery,
    role: &'static str,
    input: &OwnedBytes<Vec<u8>>,
    iterations: usize,
    warmup: usize,
    document_size_bytes: usize,
    expected_indices: Option<&[usize]>,
) -> Result<BenchmarkDetails, RewriteBenchmarkError> {
    let query_string = query.to_string();

    let compile_start = Instant::now();
    let engine = RsonpathEngine::compile_query(query)?;
    let compile_ns = compile_start.elapsed().as_nanos();

    let mut indices = Vec::new();
    engine.indices(input, &mut indices)?;

    let matches_baseline = expected_indices.is_none_or(|expected| expected == indices.as_slice());

    for _ in 0..warmup {
        engine.count(input)?;
    }

    let mut total_ns = 0_u128;

    for _ in 0..iterations {
        let start = Instant::now();
        let count = engine.count(input)?;
        let elapsed = start.elapsed().as_nanos();

        if usize::try_from(count).ok() != Some(indices.len()) {
            return Err(RewriteBenchmarkError::InconsistentCount {
                query: query_string,
                count,
                indices: indices.len(),
            });
        }

        total_ns += elapsed;
    }

    let exec_mean_ns = total_ns / iterations as u128;
    let exec_mean_s = exec_mean_ns as f64 / 1_000_000_000.0;
    let throughput_bytes_per_s = document_size_bytes as f64 / exec_mean_s;

    Ok(BenchmarkDetails {
        row: BenchmarkRow {
            query: query_string.clone(),
            role,
            query_len: query_string.len(),
            match_count: indices.len(),
            matches_baseline,
            compile_ns,
            exec_mean_ns,
            throughput_bytes_per_s,
        },
        match_indices: indices,
    })
}

#[derive(Error, Debug)]
pub enum RewriteBenchmarkError {
    #[error("--iterations must be greater than 0")]
    ZeroIterations,
    #[error("--schema-file is required unless --no-schema is set")]
    MissingSchema,
    #[error("no query matched the baseline result")]
    NoEquivalentQuery,
    #[error("failed to parse query: {0}")]
    QueryParse(#[from] rsonpath_syntax::error::ParseError),
    #[error("failed to generate rewrite candidates: {0}")]
    QueryRewrite(#[from] QueryRewriteError),
    #[error("failed to read JSON document: {0}")]
    DocumentRead(#[from] std::io::Error),
    #[error("rsonpath engine error: {0}")]
    Engine(#[from] crate::engine::error::EngineError),
    #[error("rsonpath compiler error: {0}")]
    Compiler(#[from] crate::automaton::error::CompilerError),
    #[error("query {query} returned inconsistent counts: count() = {count}, indices() = {indices}")]
    InconsistentCount {
        query: String,
        count: crate::result::MatchCount,
        indices: usize,
    },
}
