//! The standalone expressir CLI (#645): the core engines as
//! subcommands over files and stdin. Std-only argument dispatch —
//! no new dependencies. Every subcommand reads EXPRESS (or Part 21)
//! content from files or `-` (stdin) and writes to stdout.
//!
//! Subcommands:
//! - `parse <file>`      — the model-hash wire (JSON)
//! - `format <file>`     — the rendered EXPRESS text
//! - `check <file>`      — the checker findings (JSON)
//! - `resolve <file>`    — the resolved wire (JSON)
//! - `lsp-symbols <f>`   — the document-symbol rows (JSON)
//! - `lsp-inventory <f>` — the navigation inventory (JSON)
//! - `p21-parse <file>`  — the P21 inventory (JSON)
//! - `p21-write <file>`  — the verbatim Part 21 round trip
//! - `owl <file>`        — the OWL 2 Turtle rendering
//! - `json-schema <f>`   — the JSON Schema 2020-12 document
//! - `version`           — the engine version

use std::io::Read;

fn read_input(path: &str) -> Result<String, String> {
    if path == "-" {
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .map_err(|e| format!("stdin: {e}"))?;
        return Ok(buffer);
    }
    std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
}

fn wire_of(path: &str) -> Result<serde_json::Value, String> {
    let source = read_input(path)?;
    let key = if path == "-" { "stdin.exp" } else { path };
    let tree =
        expressir::ParsedTree::parse(&source).map_err(|e| format!("parse failed at {e:?}"))?;
    tree.to_model_json(key)
        .map_err(|e| format!("wire failed: {e:?}"))
}

fn run(args: &[String]) -> Result<String, String> {
    let Some(command) = args.first() else {
        return Ok(usage());
    };
    let rest = &args[1..];
    match command.as_str() {
        "version" => Ok(concat!("expressir ", env!("CARGO_PKG_VERSION"), "\n").to_string()),
        "help" | "--help" | "-h" => Ok(usage().to_string()),
        "parse" => {
            let wire = wire_of(input_path(rest, 0)?)?;
            serde_json::to_string(&wire).map_err(|e| format!("json: {e}"))
        }
        "format" => {
            let wire = wire_of(input_path(rest, 0)?)?;
            Ok(expressir::formatter::format(&wire, false))
        }
        "format-stripped" => {
            let wire = wire_of(input_path(rest, 0)?)?;
            Ok(expressir::formatter::format(&wire, true))
        }
        "check" => {
            let mut wire = wire_of(input_path(rest, 0)?)?;
            let dialect = rest
                .get(1)
                .filter(|d| !d.starts_with('-'))
                .map(String::as_str);
            let notes = expressir::checker::check(&mut wire, dialect);
            let rows: Vec<serde_json::Value> = notes
                .iter()
                .map(|note| {
                    serde_json::json!({
                        "id": note.id,
                        "severity": note.severity,
                        "schema": note.schema,
                        "message": note.message,
                    })
                })
                .collect();
            serde_json::to_string(&rows).map_err(|e| format!("json: {e}"))
        }
        "resolve" => {
            let mut wire = wire_of(input_path(rest, 0)?)?;
            let fills = expressir::checker::resolve_references(&mut wire);
            let mut wire = wire;
            wire["_gate_fills"] = serde_json::json!(fills);
            serde_json::to_string(&wire).map_err(|e| format!("json: {e}"))
        }
        "lsp-symbols" => {
            let source = read_input(input_path(rest, 0)?)?;
            expressir::lsp::symbols_json(&source)
        }
        "lsp-inventory" => {
            let source = read_input(input_path(rest, 0)?)?;
            expressir::lsp::inventory_json(&source)
        }
        "p21-parse" => {
            let source = read_input(input_path(rest, 0)?)?;
            let population = expressir::population::parse_full_population(&source)?;
            serde_json::to_string(&expressir::population::population_to_json_value(
                &population,
            ))
            .map_err(|e| format!("json: {e}"))
        }
        "p21-write" => {
            let source = read_input(input_path(rest, 0)?)?;
            let population = expressir::population::parse_full_population(&source)?;
            Ok(expressir::population::write_population(&population))
        }
        "owl" => {
            let wire = wire_of(input_path(rest, 0)?)?;
            let schema = wire
                .get("schemas")
                .and_then(|s| s.as_array())
                .and_then(|s| s.first())
                .cloned()
                .ok_or("no schema in the wire")?;
            expressir::interop::owl_json(&schema.to_string(), "")
        }
        "json-schema" => {
            let wire = wire_of(input_path(rest, 0)?)?;
            let schema = wire
                .get("schemas")
                .and_then(|s| s.as_array())
                .and_then(|s| s.first())
                .cloned()
                .ok_or("no schema in the wire")?;
            expressir::interop::json_schema_json(&schema.to_string())
        }
        other => Err(format!("unknown command '{other}' — run `expressir help`")),
    }
}

fn input_path(args: &[String], index: usize) -> Result<&str, String> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| "missing input file (or '-' for stdin)".to_string())
}

fn usage() -> String {
    r#"expressir — the EXPRESS engine (https://github.com/lutaml/expressir-rs)

USAGE:
    expressir <COMMAND> [input] [args]

INPUT:
    a file path, or '-' for stdin

COMMANDS:
    parse <input>            the model-hash wire (JSON)
    format <input>           the rendered EXPRESS text
    format-stripped <input>  the text without remarks
    check <input> [dialect]  the checker findings (JSON)
    resolve <input>          the resolved wire with _gate_fills (JSON)
    lsp-symbols <input>      the document-symbol rows (JSON)
    lsp-inventory <input>    the navigation inventory (JSON)
    p21-parse <input>        the Part 21 inventory (JSON)
    p21-write <input>        the verbatim Part 21 round trip
    owl <input>              the OWL 2 Turtle rendering
    json-schema <input>      the JSON Schema 2020-12 document
    version                  the engine version
    help                     this message
"#
    .to_string()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(text) => println!("{text}"),
        Err(message) => {
            eprintln!("expressir: {message}");
            std::process::exit(1);
        }
    }
}
