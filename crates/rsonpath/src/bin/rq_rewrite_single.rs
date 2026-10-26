use clap::Parser;
use color_eyre::eyre::Result;
use rsonpath::{rewrite_logging, rewrite_tooling::render_report};
use rsonpath_lib::query_rewrite::{
    benchmark::measure, ContextSource, PrefixToDescendantGenerator, QueryRewritePipeline, RewriteContext,
};
use std::{fs, num::NonZeroUsize, path::PathBuf};

#[derive(Parser, Debug)]
#[clap(name = "rq-rewrite-single", author, version, about)]
/// Optimize a single JSONPath query and print a result table.
struct Args {
    /// JSONPath query used as the baseline.
    query: String,
    /// Path to the JSON document used for validation and benchmarking.
    #[clap(short = 'd', long)]
    document_file: PathBuf,
    /// Path to the JSON schema file describing valid documents.
    #[clap(short, long, required_unless_present = "no_schema")]
    schema_file: Option<PathBuf>,
    /// Infer the rewrite context from the document instead of a schema file.
    #[clap(long, conflicts_with = "schema_file")]
    no_schema: bool,
    /// Number of timed execution iterations per candidate.
    #[clap(short, long, default_value = "5")]
    iterations: NonZeroUsize,
    /// Number of warm-up runs before timed execution.
    #[clap(long, default_value_t = 1)]
    warmup: usize,
    /// Rewrite pipeline logging: debug; use twice for trace detail.
    #[clap(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

fn main() -> Result<()> {
    color_eyre::install()?;

    let args = Args::parse();
    rewrite_logging::init(args.verbose)?;

    let document = fs::read_to_string(&args.document_file)?;
    let mut queries = vec![rsonpath_syntax::parse(&args.query)?];

    let generator = PrefixToDescendantGenerator;
    let context_source = if args.no_schema {
        ContextSource::DocumentFile(args.document_file.clone())
    } else {
        ContextSource::SchemaFile(args.schema_file.clone().unwrap())
    };
    
    let context = RewriteContext::build(context_source)?;

    let pipeline = QueryRewritePipeline::new(vec![&generator], &context);
    let rewritten = pipeline.optimize_query(&queries[0])?;
    queries.extend(rewritten);
    let document_bytes = document.as_bytes().to_vec();

    let results = measure(document_bytes, &queries, args.iterations, args.warmup)?;

    println!(
        "{}",
        render_report(
            &args.query,
            args.schema_file.as_deref(),
            &args.document_file,
            args.iterations,
            args.warmup,
            &results,
        )
    );

    Ok(())
}
