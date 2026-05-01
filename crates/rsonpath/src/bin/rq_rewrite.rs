use clap::Parser;
use color_eyre::eyre::{Result, WrapErr as _};
use rsonpath_lib::query_rewrite::optimizer::optimize_query_with_schema_file;
use rsonpath_syntax::JsonPathQuery;

#[derive(Parser, Debug)]
#[clap(name = "rq-rewrite", author, version, about)]
/// Generate schema-aware equivalent JSONPath rewrites.
struct Args {
    /// JSONPath query to rewrite.
    query: String,
    /// Path to the JSON schema file describing valid documents.
    #[clap(short, long)]
    schema_file: String,
}

fn main() -> Result<()> {
    color_eyre::install()?;

    let args = Args::parse();
    let rewritten = optimize_query_with_schema_file(&args.query, &args.schema_file)
        .wrap_err("Failed to generate query rewrites.")?;

    println!("{}", render_text(&args.query, &args.schema_file, &rewritten));

    Ok(())
}

fn render_text(query: &str, schema_file: &str, rewritten: &[JsonPathQuery]) -> String {
    let mut output = format!(
        "Input query: {query}\nSchema file: {schema_file}\nEquivalent rewrites: {}",
        rewritten.len()
    );

    for (idx, candidate) in rewritten.iter().enumerate() {
        output.push_str(&format!("\n{}. {}", idx + 1, candidate));
    }

    output
}

#[cfg(test)]
mod tests {
    use super::render_text;

    #[test]
    fn text_output_includes_header_and_candidates() {
        let candidates = vec![
            rsonpath_syntax::parse("$['content'][*]['title']").unwrap(),
            rsonpath_syntax::parse("$..['title']").unwrap(),
        ];

        let rendered = render_text("$.content[*].title", "schema.json", &candidates);

        assert!(rendered.contains("Input query: $.content[*].title"));
        assert!(rendered.contains("Schema file: schema.json"));
        assert!(rendered.contains("Equivalent rewrites: 2"));
        assert!(rendered.contains("1. $['content'][*]['title']"));
        assert!(rendered.contains("2. $..['title']"));
    }
}
