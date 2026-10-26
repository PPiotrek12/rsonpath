use rsonpath_lib::query_rewrite::benchmark::{BenchResult, Outcome};
use std::{num::NonZeroUsize, path::Path};

/// Render a measurement table for a single-query rewrite run.
///
/// Index 0 is the baseline. Failed candidates keep a row so they stay visible;
/// numeric columns are blank for those rows.
#[inline]
#[must_use]
pub fn render_report(
    query: &str,
    schema_file: Option<&Path>,
    document_file: &Path,
    iterations: NonZeroUsize,
    warmup: usize,
    outcomes: &[Outcome],
) -> String {
    let baseline_throughput = match outcomes.first() {
        Some(Outcome::Measured(row)) => row.throughput_bytes_per_s,
        _ => 1.0,
    };
    let generated_candidates = outcomes.len().saturating_sub(1);

    let mut table_rows = Vec::with_capacity(outcomes.len() + 1);
    table_rows.push(vec![
        String::from("Query"),
        String::from("Len"),
        String::from("Matches"),
        String::from("Status"),
        String::from("Compile ms"),
        String::from("Mean ms"),
        String::from("Throughput GB/s"),
        String::from("Speedup"),
        String::from("Role"),
    ]);

    for (idx, outcome) in outcomes.iter().enumerate() {
        let role = if idx == 0 { "baseline" } else { "rewrite" };
        table_rows.push(match outcome {
            Outcome::Measured(row) => measured_row(row, baseline_throughput, role),
            Outcome::Failed { query, error } => failed_row(query, error.to_string(), role),
        });
    }

    format!(
        "Baseline query: {query}\nSchema file: {schema}\nDocument file: {}\nGenerated candidates: {generated_candidates}\nIterations per candidate: {iterations}\nWarm-up runs: {warmup}\nValidation: exact match-index equality against the baseline query\n\n{}",
        document_file.display(),
        format_table(&table_rows),
        schema = schema_file.map_or_else(|| "None".to_string(), |path| path.display().to_string()),
    )
}

fn measured_row(row: &BenchResult, baseline_throughput: f64, role: &str) -> Vec<String> {
    let speedup = row.throughput_bytes_per_s / baseline_throughput;
    vec![
        truncate_query(&row.query, 56),
        row.query.len().to_string(),
        row.match_count.to_string(),
        if row.matches_baseline {
            String::from("OK")
        } else {
            String::from("DIFF")
        },
        format_ms(row.compile_ns),
        format_ms(row.exec_mean_ns),
        format!("{:.2}", row.throughput_bytes_per_s / 1_000_000_000.0),
        format!("{speedup:.2}x"),
        role.to_string(),
    ]
}

fn failed_row(query: &str, error: String, role: &str) -> Vec<String> {
    vec![
        truncate_query(query, 56),
        query.len().to_string(),
        String::from("—"),
        truncate_query(&format!("FAIL: {error}"), 24),
        String::from("—"),
        String::from("—"),
        String::from("—"),
        String::from("—"),
        role.to_string(),
    ]
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

        assert!(rendered.contains("A      | B"), "header should pad to the wider body cell");
        assert!(rendered.contains("longer | x"), "body row should keep the longer first cell");
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
