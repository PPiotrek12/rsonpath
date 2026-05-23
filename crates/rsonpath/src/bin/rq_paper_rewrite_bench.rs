use clap::Parser;
use color_eyre::eyre::{Result, WrapErr as _};
use rsonpath::rewrite_logging;
use rsonpath::rewrite_tooling::{
    benchmark_fixed_query, select_best_rewrite, BenchmarkRow, FixedQueryBenchmarkConfig, RewriteBenchmarkConfig,
};
use std::{fs, path::PathBuf};

#[derive(Parser, Debug)]
#[clap(name = "rq-paper-rewrite-bench", author, version, about)]
/// Benchmark Table 4 paper queries against automatic and manual rewrites.
struct Args {
    /// Root directory containing benchmark datasets.
    #[clap(long, default_value = "crates/rsonpath-benchmarks/data")]
    data_dir: PathBuf,
    /// Output CSV path.
    #[clap(short, long, default_value = "target/paper-rewrite-bench.csv")]
    output: PathBuf,
    /// Number of timed execution iterations per query.
    #[clap(short, long, default_value_t = 20)]
    iterations: usize,
    /// Number of warm-up runs before timed execution.
    #[clap(long, default_value_t = 1)]
    warmup: usize,
    /// Continue when a dataset file is missing.
    #[clap(long)]
    skip_missing: bool,
    /// Rewrite pipeline logging: debug; use twice for trace detail.
    #[clap(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

#[derive(Clone, Copy)]
struct PaperQuery {
    id: &'static str,
    dataset: &'static str,
    document: &'static str,
    original: &'static str,
    manual: Option<&'static str>,
}

#[derive(Clone)]
struct VariantResult {
    id: &'static str,
    dataset: &'static str,
    variant: &'static str,
    auto_candidate_rank: Option<usize>,
    row: BenchmarkRow,
    speedup_vs_original: f64,
}

const PAPER_TABLE_4_QUERIES: &[PaperQuery] = &[
    PaperQuery {
        id: "B1",
        dataset: "bestbuy",
        document: "pison/bestbuy_large_record.json",
        original: "$.products[*].categoryPath[*].id",
        manual: Some("$..categoryPath..id"),
    },
    PaperQuery {
        id: "B2",
        dataset: "bestbuy",
        document: "pison/bestbuy_large_record.json",
        original: "$.products[*].videoChapters[*].chapter",
        manual: Some("$..videoChapters..chapter"),
    },
    PaperQuery {
        id: "B3",
        dataset: "bestbuy",
        document: "pison/bestbuy_large_record.json",
        original: "$.products[*].videoChapters",
        manual: Some("$..videoChapters"),
    },
    PaperQuery {
        id: "G1",
        dataset: "google_map",
        document: "pison/google_map_large_record.json",
        original: "$[*].routes[*].legs[*].steps[*].distance.text",
        manual: None,
    },
    PaperQuery {
        id: "G2",
        dataset: "google_map",
        document: "pison/google_map_large_record.json",
        original: "$[*].available_travel_modes",
        manual: Some("$..available_travel_modes"),
    },
    PaperQuery {
        id: "N1",
        dataset: "nspl",
        document: "pison/nspl_large_record.json",
        original: "$.meta.view.columns[*].name",
        manual: None,
    },
    PaperQuery {
        id: "N2",
        dataset: "nspl",
        document: "pison/nspl_large_record.json",
        original: "$.data[*][*][*]",
        manual: None,
    },
    PaperQuery {
        id: "T1",
        dataset: "twitter",
        document: "pison/twitter_large_record.json",
        original: "$[*].entities.urls[*].url",
        manual: None,
    },
    PaperQuery {
        id: "T2",
        dataset: "twitter",
        document: "pison/twitter_large_record.json",
        original: "$[*].text",
        manual: None,
    },
    PaperQuery {
        id: "W1",
        dataset: "walmart",
        document: "pison/walmart_large_record.json",
        original: "$.items[*].bestMarketplacePrice.price",
        manual: Some("$..bestMarketplacePrice.price"),
    },
    PaperQuery {
        id: "W2",
        dataset: "walmart",
        document: "pison/walmart_large_record.json",
        original: "$.items[*].name",
        manual: Some("$..name"),
    },
    PaperQuery {
        id: "Wi",
        dataset: "wikidata",
        document: "pison/wiki_large_record.json",
        original: "$[*].claims.P150[*].mainsnak.property",
        manual: Some("$..P150..mainsnak.property"),
    },
];

fn main() -> Result<()> {
    color_eyre::install()?;

    let args = Args::parse();
    rewrite_logging::init(args.verbose)?;

    if args.iterations == 0 {
        color_eyre::eyre::bail!("--iterations must be greater than 0");
    }

    let mut results = Vec::new();
    for case in PAPER_TABLE_4_QUERIES {
        let document_file = args.data_dir.join(case.document);
        if !document_file.exists() {
            let msg = format!("missing dataset for {}: {}", case.id, document_file.display());
            if args.skip_missing {
                eprintln!("{msg}; skipping");
                continue;
            }
            color_eyre::eyre::bail!(msg);
        }

        eprintln!("Benchmarking {} on {}...", case.id, case.dataset);
        let document_file = document_file.to_string_lossy().into_owned();
        let selection = select_best_rewrite(&RewriteBenchmarkConfig {
            query: case.original.to_string(),
            schema_file: None,
            no_schema: true,
            document_file: document_file.clone(),
            iterations: args.iterations,
            warmup: args.warmup,
        })
        .wrap_err_with(|| format!("failed to select automatic rewrite for {}", case.id))?;

        let original = selection
            .report
            .rows
            .iter()
            .find(|row| row.role == "original")
            .cloned()
            .ok_or_else(|| color_eyre::eyre::eyre!("missing original row for {}", case.id))?;
        let original_throughput = original.throughput_bytes_per_s;

        push_result(
            &mut results,
            case,
            "paper_original",
            None,
            original,
            original_throughput,
        );
        push_result(
            &mut results,
            case,
            "auto_best",
            None,
            selection.row.clone(),
            original_throughput,
        );

        for (rank, row) in selection
            .report
            .rows
            .iter()
            .filter(|row| row.role == "rewrite")
            .cloned()
            .enumerate()
        {
            push_result(
                &mut results,
                case,
                "auto_candidate",
                Some(rank + 1),
                row,
                original_throughput,
            );
        }

        if let Some(manual) = case.manual {
            let manual = benchmark_fixed_query(&FixedQueryBenchmarkConfig {
                baseline_query: case.original.to_string(),
                query: manual.to_string(),
                role: "paper_manual",
                document_file: document_file.clone(),
                iterations: args.iterations,
                warmup: args.warmup,
            })
            .wrap_err_with(|| format!("failed to benchmark manual rewrite for {}", case.id))?;
            push_result(&mut results, case, "paper_manual", None, manual, original_throughput);
        }
    }

    write_csv(&args.output, &results)?;
    println!("{}", render_terminal_table(&results));
    println!("CSV written to {}", args.output.display());

    Ok(())
}

fn push_result(
    results: &mut Vec<VariantResult>,
    case: &'static PaperQuery,
    variant: &'static str,
    auto_candidate_rank: Option<usize>,
    row: BenchmarkRow,
    original_throughput: f64,
) {
    results.push(VariantResult {
        id: case.id,
        dataset: case.dataset,
        variant,
        auto_candidate_rank,
        speedup_vs_original: row.throughput_bytes_per_s / original_throughput,
        row,
    });
}

fn write_csv(path: &PathBuf, results: &[VariantResult]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).wrap_err_with(|| format!("failed to create {}", parent.display()))?;
    }

    let mut output = String::from(
        "id,dataset,variant,auto_candidate_rank,query,query_len,match_count,matches_baseline,compile_ns,exec_mean_ns,throughput_bytes_per_s,speedup_vs_original\n",
    );
    for result in results {
        output.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{:.6},{:.6}\n",
            csv_escape(result.id),
            csv_escape(result.dataset),
            csv_escape(result.variant),
            result
                .auto_candidate_rank
                .map_or_else(String::new, |rank| rank.to_string()),
            csv_escape(&result.row.query),
            result.row.query_len,
            result.row.match_count,
            result.row.matches_baseline,
            result.row.compile_ns,
            result.row.exec_mean_ns,
            result.row.throughput_bytes_per_s,
            result.speedup_vs_original,
        ));
    }

    fs::write(path, output).wrap_err_with(|| format!("failed to write {}", path.display()))
}

fn render_terminal_table(results: &[VariantResult]) -> String {
    let mut rows = vec![vec![
        String::from("ID"),
        String::from("Dataset"),
        String::from("Variant"),
        String::from("Mean ms"),
        String::from("Throughput GB/s"),
        String::from("Speedup"),
        String::from("Query"),
    ]];

    for result in results {
        rows.push(vec![
            result.id.to_string(),
            result.dataset.to_string(),
            result.variant.to_string(),
            format!("{:.3}", result.row.exec_mean_ns as f64 / 1_000_000.0),
            format!("{:.2}", result.row.throughput_bytes_per_s / 1_000_000_000.0),
            format!("{:.2}x", result.speedup_vs_original),
            truncate_query(&result.row.query, 64),
        ]);
    }

    format_table(&rows)
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
            output.push_str(
                &widths
                    .iter()
                    .map(|width| "-".repeat(*width))
                    .collect::<Vec<_>>()
                    .join("-+-"),
            );
            output.push('\n');
        }
    }
    output
}

fn truncate_query(query: &str, max_len: usize) -> String {
    if query.len() <= max_len {
        return query.to_string();
    }

    let keep = max_len.saturating_sub(3);
    format!("{}...", &query[..keep])
}

fn csv_escape(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}
