use clap::{ArgGroup, Parser};
use color_eyre::eyre::Result;
use rsonpath_syntax::JsonPathQuery;
use rsonpath_lib::query_rewrite::{ContextSource, PrefixToDescendantGenerator, QueryRewritePipeline, RewriteContext};

#[derive(Parser, Debug)]
#[clap(name = "rq-rewrite", author, version, about)]
#[clap(group(
    ArgGroup::new("context")
        .required(true)
        .args(["schema_file", "document_file"])
))]
/// Generate schema-aware equivalent JSONPath rewrites.
struct Args {
    /// JSONPath query to rewrite.
    query: String,
    /// Path to the JSON document used as the rewrite context.
    #[clap(short, long)]
    document_file: Option<String>,

    /// Path to the JSON schema file describing valid documents.
    #[clap(short, long)]
    schema_file: Option<String>,
}

fn main() -> Result<()> {
    color_eyre::install()?;

    let args = Args::parse();

    let context_source = match (args.schema_file, args.document_file) {
        (Some(schema_file), _) => ContextSource::SchemaFile(schema_file),
        (_, Some(document_file)) => ContextSource::DocumentFile(document_file),
        (None, None) => unreachable!("clap group `context` is required and exclusive"),
    };

    let generator = PrefixToDescendantGenerator;
    let context = RewriteContext::build(context_source)?;
    let pipeline = QueryRewritePipeline::new(vec![&generator], &context);
    let query = rsonpath_syntax::parse(&args.query)?;

    let rewritten = pipeline.optimize_query(&query)?;
    println!("{}", render_text(&query.to_string(), context.source(), &rewritten));

    Ok(())
}

fn render_text(query: &str, context: &ContextSource, rewritten: &[JsonPathQuery]) -> String {
    let context_str = match context {
        ContextSource::SchemaFile(path) => format!("schema file: {path}"),
        ContextSource::DocumentFile(path) => format!("document file: {path}"),
    };
    let mut output = format!(
        "Input query: {query}\nContext file: {context_str}\nEquivalent rewrites: {}",
        rewritten.len()
    );

    for (idx, candidate) in rewritten.iter().enumerate() {
        output.push_str(&format!("\n{}. {}", idx + 1, candidate));
    }

    output
}
