use clap::Parser;
use color_eyre::eyre::{Result, WrapErr as _};
use rsonpath::rewrite_logging;
use rsonpath::rewrite_suite::Suite;
use rsonpath_lib::query_rewrite::{ContextSource, PrefixToDescendantGenerator, QueryRewritePipeline, benchmark::measure, context::ContextCacheStore};
use rsonpath::rewrite_tooling::render_report;
use std::{fs, num::NonZeroUsize, path::PathBuf};

#[derive(Parser, Debug)]
#[clap(name = "rq-rewrite-bench", author, version, about)]
/// Benchmark a JSON suite of queries against automatic and manual rewrites.
struct Args {
    /// Path to the suite JSON file.
    #[clap(long, default_value = "crates/rsonpath-benchmarks/queries/paper-table-4.json")]
    suite: PathBuf,
    /// Override the suite's dataset root directory.
    #[clap(long)]
    data_dir: Option<PathBuf>,
    /// Output CSV path.
    #[clap(short, long, default_value = "target/paper-rewrite-bench.csv")]
    output: PathBuf,
    /// Override the suite's timed iteration count.
    #[clap(short, long, default_value = "5")]
    iterations: NonZeroUsize,
    /// Override the suite's warm-up run count.
    #[clap(long, default_value = "2")]
    warmup: usize,
    /// Continue when a dataset file is missing.
    #[clap(long)]
    skip_missing: bool,
    /// Logging: info from this binary is always on; -v debug / -vv trace for the rewrite pipeline.
    #[clap(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

fn main() -> Result<()> {
    color_eyre::install()?;

    let args = Args::parse();
    rewrite_logging::init_for_binary(args.verbose, module_path!())?;

    let suite = Suite::load(&args.suite)
        .wrap_err_with(|| format!("failed to load suite {}", args.suite.display()))?;


    let mut context_store = ContextCacheStore::new(); 

    for case in &suite.cases { 
        let data_dir = args.data_dir.as_deref().unwrap_or(&suite.data_dir);
        let document_path = data_dir.join(&case.document);
        let context_source = match case.schema.as_deref() {
            Some(schema) => ContextSource::SchemaFile(data_dir.join(schema)),
            None => ContextSource::DocumentFile(document_path.clone()),
        };

        let context = if let Some(context) = context_store.get(&context_source) {
            context
        } else {
            log::info!("Building context for {}", context_source.path().display());
            context_store.build_and_insert(context_source)?
        };

        let document = fs::read_to_string(&document_path)?;
        let document_bytes = document.as_bytes().to_vec();
        drop(document);

        let generator = PrefixToDescendantGenerator;
        let pipeline = QueryRewritePipeline::new(vec![&generator], context);
        let mut queries = vec![rsonpath_syntax::parse(&case.query)?];
        let rewritten = pipeline.optimize_query(&queries[0])?;
        queries.extend(rewritten);

        drop(pipeline);

        let results = measure(document_bytes, &queries, args.iterations, args.warmup)?;

        println!(
            "{}",
            render_report(
                &case.query,
                case.schema.as_ref().map(|_| context.source().path()),
                &document_path,
                args.iterations,
                args.warmup,
                &results,
            )
        );
    }

    Ok(())
}
