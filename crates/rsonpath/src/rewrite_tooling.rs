pub use rsonpath_lib::query_rewrite::benchmark::{
    benchmark_fixed_query, best_equivalent_row, run_benchmark, select_best_rewrite, BenchmarkRow, BestRewriteSelection,
    FixedQueryBenchmarkConfig, RewriteBenchmarkConfig, RewriteBenchmarkError, RewriteBenchmarkReport,
};

pub fn render_report(config: &RewriteBenchmarkConfig, report: &RewriteBenchmarkReport) -> String {
    let baseline_throughput = report
        .rows
        .iter()
        .find(|row| row.role == "original")
        .map(|row| row.throughput_bytes_per_s)
        .unwrap_or(1.0);

    let mut table_rows = Vec::with_capacity(report.rows.len() + 1);
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

    for row in &report.rows {
        let speedup = row.throughput_bytes_per_s / baseline_throughput;
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
            format!("{:.2}", row.throughput_bytes_per_s / 1_000_000_000.0),
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
