use clap::{Parser, ValueEnum};
use color_eyre::eyre::Result;
use rsonpath::rewrite_tooling::{render_report, select_best_rewrite, RewriteBenchmarkConfig};

#[derive(Parser, Debug)]
#[clap(name = "rq-rewrite-best", author, version, about)]
/// Select the fastest validated rewrite for a JSONPath query on a concrete document.
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
    /// Output format.
    #[clap(long, value_enum, default_value_t = OutputFormat::Query)]
    format: OutputFormat,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum OutputFormat {
    /// Print only the selected query.
    Query,
    /// Print the full validation table followed by the selected query.
    Report,
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
    let selection = select_best_rewrite(&config)?;

    match args.format {
        OutputFormat::Query => println!("{}", selection.query),
        OutputFormat::Report => {
            println!("{}", render_report(&config, &selection.report));
            println!("Best equivalent query: {}", selection.query);
        }
    }

    Ok(())
}
