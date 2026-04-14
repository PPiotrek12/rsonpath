use query_rewrite::json_schema_parser::from_file;
use rsonpath::query_rewrite;

fn main() {
    match from_file("examples/example_schema.json") {
        Ok(automaton) => {
            dbg!(automaton);
        }
        Err(e) => {
            dbg!(e);
        }
    }
}
