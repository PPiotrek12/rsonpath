use clap::Parser;
use color_eyre::eyre::Result;
use rsonpath::rewrite_tooling::{render_report, run_benchmark, RewriteBenchmarkConfig};

#[derive(Parser, Debug)]
#[clap(name = "rq-rewrite-validate", author, version, about)]
/// Benchmark and validate schema-aware query rewrites on a concrete document.
struct Args {
    /// JSONPath query used as the baseline.
    query: String,
    /// Path to the JSON schema file describing valid documents.
    #[clap(short, long, required_unless_present = "no_schema")]
    schema_file: Option<String>,
    /// Boolean flag whether we want to infer schema from document.
    #[clap(long, conflicts_with = "schema_file")]
    no_schema: bool,
    /// Path to the JSON document used for validation and benchmarking.
    #[clap(short = 'd', long)]
    document_file: String,
    /// Number of timed execution iterations per candidate.
    #[clap(short, long, default_value_t = 10)]
    iterations: usize,
    /// Number of warm-up runs before timed execution.
    #[clap(long, default_value_t = 1)]
    warmup: usize,
}

fn main() -> Result<()> {
    color_eyre::install()?;

    let args = Args::parse();
    let config = RewriteBenchmarkConfig {
        query: args.query,
        schema_file: args.schema_file,
        no_schema: args.no_schema,
        document_file: args.document_file,
        iterations: args.iterations,
        warmup: args.warmup,
    };
    let report = run_benchmark(&config)?;

    println!("{}", render_report(&config, &report));

    Ok(())
}
