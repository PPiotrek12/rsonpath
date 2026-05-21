use color_eyre::eyre::{OptionExt as _, Result, WrapErr as _};
use rsonpath_lib::{
    engine::{Compiler as _, Engine as _, RsonpathEngine},
    input::OwnedBytes,
    query_rewrite::{optimize_query_with_schema_file, optimize_query_without_schema_file},
};
use rsonpath_syntax::JsonPathQuery;
use std::{cmp, fs, time::Instant};

#[derive(Debug)]
pub struct RewriteBenchmarkConfig {
    pub query: String,
    pub schema_file: Option<String>,
    pub no_schema: bool,
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
    pub exec_min_ns: u128,
    pub exec_max_ns: u128,
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

pub fn run_benchmark(config: &RewriteBenchmarkConfig) -> Result<RewriteBenchmarkReport> {
    if config.iterations == 0 {
        color_eyre::eyre::bail!("--iterations must be greater than 0");
    }

    if !config.no_schema && config.schema_file.is_none() {
        color_eyre::eyre::bail!("--schema-file is required unless --no-schema is set");
    }

    let baseline_query = rsonpath_syntax::parse(&config.query).wrap_err("Failed to parse the baseline query.")?;
    let candidates = if let Some(schema) = &config.schema_file {
        optimize_query_with_schema_file(&config.query, schema)
            .wrap_err("Failed to generate rewrite candidates from the schema.")?
    } else {
        optimize_query_without_schema_file(&config.query, &config.document_file)?
    };
    let generated_candidates = candidates.len();
    let document = fs::read(&config.document_file).wrap_err("Failed to read the JSON document.")?;
    let input = OwnedBytes::new(document);

    let baseline = benchmark_query(&baseline_query, "original", &input, config, None)
        .wrap_err("Failed while benchmarking the baseline query.")?;
    let expected_indices = baseline.match_indices.clone();

    let mut rows = Vec::with_capacity(candidates.len() + 1);
    rows.push(baseline.row);

    for candidate in candidates {
        if candidate == baseline_query {
            continue;
        }

        let details = benchmark_query(&candidate, "rewrite", &input, config, Some(&expected_indices))
            .wrap_err_with(|| format!("Failed while benchmarking candidate {candidate}."))?;
        rows.push(details.row);
    }

    let (baseline_row, mut rewrites): (Vec<_>, Vec<_>) = rows.into_iter().partition(|row| row.role == "original");
    rewrites.sort_by_key(|row| row.exec_mean_ns);

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
        .min_by_key(|row| row.exec_mean_ns)
}

pub fn select_best_rewrite(config: &RewriteBenchmarkConfig) -> Result<BestRewriteSelection> {
    let report = run_benchmark(config)?;
    let row = best_equivalent_row(&report)
        .cloned()
        .ok_or_eyre("no query matched the baseline result")?;

    Ok(BestRewriteSelection {
        query: row.query.clone(),
        row,
        report,
    })
}

pub fn render_report(config: &RewriteBenchmarkConfig, report: &RewriteBenchmarkReport) -> String {
    let baseline_exec_ns = report
        .rows
        .iter()
        .find(|row| row.role == "original")
        .map(|row| row.exec_mean_ns)
        .unwrap_or(1);

    let mut table_rows = Vec::with_capacity(report.rows.len() + 1);
    table_rows.push(vec![
        String::from("Query"),
        String::from("Len"),
        String::from("Matches"),
        String::from("Status"),
        String::from("Compile ms"),
        String::from("Mean ms"),
        String::from("Min ms"),
        String::from("Max ms"),
        String::from("Speedup"),
        String::from("Role"),
    ]);

    for row in &report.rows {
        let speedup = baseline_exec_ns as f64 / row.exec_mean_ns as f64;
        table_rows.push(vec![
            truncate_query(&row.query, 56),
            row.query_len.to_string(),
            row.match_count.to_string(),
            if row.matches_baseline {
                String::from("OK")
            } else {
                String::from("DIFF")
            },
            format_ms(row.compile_ns),
            format_ms(row.exec_mean_ns),
            format_ms(row.exec_min_ns),
            format_ms(row.exec_max_ns),
            format!("{speedup:.2}x"),
            row.role.to_string(),
        ]);
    }

    let table = format_table(&table_rows);

    format!(
        "Baseline query: {}\nSchema file: {:?}\nDocument file: {}\nGenerated candidates: {}\nIterations per candidate: {}\nWarm-up runs: {}\nValidation: exact match-index equality against the baseline query\n\n{}",
        config.query,
        config.schema_file,
        config.document_file,
        report.generated_candidates,
        config.iterations,
        config.warmup,
        table
    )
}

fn benchmark_query(
    query: &JsonPathQuery,
    role: &'static str,
    input: &OwnedBytes<Vec<u8>>,
    config: &RewriteBenchmarkConfig,
    expected_indices: Option<&[usize]>,
) -> Result<BenchmarkDetails> {
    let query_string = query.to_string();

    let compile_start = Instant::now();
    let engine =
        RsonpathEngine::compile_query(query).wrap_err_with(|| format!("Failed to compile query {query_string}."))?;
    let compile_ns = compile_start.elapsed().as_nanos();

    let mut indices = Vec::new();
    engine
        .indices(input, &mut indices)
        .wrap_err_with(|| format!("Failed to collect match indices for query {query_string}."))?;

    let matches_baseline = expected_indices.is_none_or(|expected| expected == indices.as_slice());

    for _ in 0..config.warmup {
        engine
            .count(input)
            .wrap_err_with(|| format!("Warm-up execution failed for query {query_string}."))?;
    }

    let mut total_ns = 0_u128;
    let mut min_ns = u128::MAX;
    let mut max_ns = 0_u128;

    for _ in 0..config.iterations {
        let start = Instant::now();
        let count = engine
            .count(input)
            .wrap_err_with(|| format!("Timed execution failed for query {query_string}."))?;
        let elapsed = start.elapsed().as_nanos();

        if usize::try_from(count).ok() != Some(indices.len()) {
            color_eyre::eyre::bail!(
                "query {query_string} returned inconsistent counts: count() = {count}, indices() = {}",
                indices.len()
            );
        }

        total_ns += elapsed;
        min_ns = cmp::min(min_ns, elapsed);
        max_ns = cmp::max(max_ns, elapsed);
    }

    let exec_mean_ns = total_ns / config.iterations as u128;

    Ok(BenchmarkDetails {
        row: BenchmarkRow {
            query: query_string.clone(),
            role,
            query_len: query_string.len(),
            match_count: indices.len(),
            matches_baseline,
            compile_ns,
            exec_mean_ns,
            exec_min_ns: min_ns,
            exec_max_ns: max_ns,
        },
        match_indices: indices,
    })
}

fn format_table(rows: &[Vec<String>]) -> String {
    let column_count = rows.first().map_or(0, Vec::len);
    let widths: Vec<usize> = (0..column_count)
        .map(|column| {
            rows.iter()
                .map(|row| row.get(column).map_or(0, |cell| cell.len()))
                .max()
                .unwrap_or(0)
        })
        .collect();

    let mut output = String::new();

    for (row_idx, row) in rows.iter().enumerate() {
        let line = row
            .iter()
            .enumerate()
            .map(|(idx, cell)| format!("{cell:<width$}", width = widths[idx]))
            .collect::<Vec<_>>()
            .join(" | ");
        output.push_str(&line);
        output.push('\n');

        if row_idx == 0 {
            let separator = widths
                .iter()
                .map(|width| "-".repeat(*width))
                .collect::<Vec<_>>()
                .join("-+-");
            output.push_str(&separator);
            output.push('\n');
        }
    }

    output
}

fn format_ms(ns: u128) -> String {
    format!("{:.3}", ns as f64 / 1_000_000.0)
}

fn truncate_query(query: &str, max_len: usize) -> String {
    if query.len() <= max_len {
        return query.to_string();
    }

    let keep = max_len.saturating_sub(3);
    format!("{}...", &query[..keep])
}

#[cfg(test)]
mod tests {
    use super::{format_table, truncate_query};

    #[test]
    fn table_renderer_aligns_columns() {
        let rendered = format_table(&[vec!["A".into(), "B".into()], vec!["longer".into(), "x".into()]]);

        assert!(rendered.contains("A      | B"));
        assert!(rendered.contains("longer | x"));
    }

    #[test]
    fn truncation_keeps_short_queries_unchanged() {
        assert_eq!(truncate_query("$.a", 10), "$.a");
    }

    #[test]
    fn truncation_adds_ellipsis_for_long_queries() {
        assert_eq!(truncate_query("$['content'][*]['title']", 12), "$['conten...");
    }
}
