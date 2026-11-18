//! Rust-vs-Ruby formatter-flavor parity (expressir#637, slice 2).
//!
//! The mixin faces over the source formatter — HyperlinkFormatter
//! (the stepmod markup), AdocHyperlinkFormatter (AsciiDoc xrefs) and
//! SchemaHeadFormatter (head-only schemas) — driven on BOTH lanes
//! over a RESOLVED wire (base_path present). The Ruby lane hydrates
//! the rs wire, resolves references
//! (`ResolveReferencesModelVisitor`) and runs the three formatter
//! classes at file level plus the schema-level faces; the Rust lane
//! replays `checker::resolve_references` + `formatter::format_flavored`.
//! Byte-equal output. Skipped without `EXPRESSIR_DIR`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Map, Value};

const RUBY_HARNESS: &str = r####"
require "json"
root = ARGV[0]
$LOAD_PATH.unshift(File.join(root, "lib"))
require "expressir"

request = JSON.parse($stdin.read)
out = {}
errors = {}
request["files"].each do |path, wire|
  begin
    model = Expressir::Model::ExpFile.from_hash(wire)
    model.wire_parents
    # the wire arrives RESOLVED (rs fills); the flavor gate owns the
    # formatting differential — resolve parity is resolve_parity's
    row = {
      "file_source" => Expressir::Express::Formatter.new.format(model),
      "file_hyperlink" => Expressir::Express::SourceFormatter.format(model),
      "file_adoc" => Expressir::Express::AdocSourceFormatter.format(model),
    }
    schema = model.schemas.first
    if schema
      row["schema_plain_head"] =
        Expressir::Express::SchemaPlainSourceFormatter.format(schema)
      row["schema_hyperlinked_head"] =
        Expressir::Express::SchemaSourceFormatter.format(schema)
      row["schema_adoc"] =
        Expressir::Express::AdocSourceFormatter.format(schema)
    end
    out[path] = row
  rescue StandardError => e
    errors[path] = "#{e.class}: #{e.message}"
  end
end
puts JSON.generate({ "texts" => out, "errors" => errors })
"####;

fn expressir_dir() -> Option<PathBuf> {
    std::env::var_os("EXPRESSIR_DIR").map(PathBuf::from)
}

fn collect_exp_files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_exp_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "exp") {
            out.push(path);
        }
    }
}

type LaneMaps = Result<(Map<String, Value>, Map<String, Value>), String>;

fn ruby_faces(expressir_dir: &Path, wires: &Map<String, Value>) -> LaneMaps {
    let request = json!({ "files": wires });
    let mut child = Command::new("bundle")
        .arg("exec")
        .arg("ruby")
        .arg("-e")
        .arg(RUBY_HARNESS)
        .arg(expressir_dir)
        .current_dir(expressir_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("bundle/ruby unavailable: {e}"))?;
    {
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(request.to_string().as_bytes())
            .map_err(|e| format!("write: {e}"))?;
    }
    let output = child.wait_with_output().map_err(|e| format!("wait: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "ruby harness failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let parsed: Value = serde_json::from_slice(&output.stdout).map_err(|e| format!("json: {e}"))?;
    Ok((
        parsed["texts"].as_object().cloned().unwrap_or_default(),
        parsed["errors"].as_object().cloned().unwrap_or_default(),
    ))
}

/// (gate key, schema path or "" for the file face, rs options)
fn faces() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        ("file_source", "", json!({ "flavor": "source" })),
        ("file_hyperlink", "", json!({ "flavor": "hyperlink" })),
        ("file_adoc", "", json!({ "flavor": "adoc" })),
        (
            "schema_plain_head",
            "schema",
            json!({ "flavor": "source", "head_only": true }),
        ),
        (
            "schema_hyperlinked_head",
            "schema",
            json!({ "flavor": "hyperlink", "head_only": true }),
        ),
        ("schema_adoc", "schema", json!({ "flavor": "adoc" })),
    ]
}

fn first_diff(actual: &str, expected: &str) -> usize {
    actual
        .as_bytes()
        .iter()
        .zip(expected.as_bytes().iter())
        .position(|(a, b)| a != b)
        .unwrap_or(actual.len().min(expected.len()))
}

#[test]
fn formatter_flavor_parity() {
    // The big corpus trees recurse deeply — run on a fat stack.
    let handle = std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(run_parity)
        .expect("fat-stack thread");
    handle.join().expect("parity run");
}

fn run_parity() {
    let Some(expressir_dir) = expressir_dir() else {
        eprintln!("EXPRESSIR_DIR unset — skipping the flavor parity run");
        return;
    };
    let mut files = Vec::new();
    collect_exp_files(&expressir_dir, &mut files);
    files.sort();
    files.dedup();
    assert!(!files.is_empty(), "no .exp fixtures found");

    let mut wires: Map<String, Value> = Map::new();
    for path in &files {
        let key = path.to_string_lossy().into_owned();
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(mut tree_wire) =
            expressir::ParsedTree::parse(&source).map(|t| t.to_model_json(&key).unwrap())
        else {
            eprintln!("skip (rust lane cannot parse): {key}");
            continue;
        };
        expressir::checker::resolve_references(&mut tree_wire);
        wires.insert(key, tree_wire);
    }

    let (ruby, errors) = ruby_faces(&expressir_dir, &wires).expect("ruby flavor lane");
    assert!(errors.is_empty(), "ruby harness raised: {errors:?}");

    let face_specs = faces();
    let mut mismatches: Vec<String> = Vec::new();
    let mut compared = 0usize;
    for (key, wire) in &wires {
        let Some(row) = ruby.get(key) else {
            mismatches.push(format!("{key}: missing from the ruby lane"));
            continue;
        };
        let schema_path = {
            let count = wire
                .get("schemas")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            if count > 0 {
                "/schemas/0".to_string()
            } else {
                String::new()
            }
        };
        for (face, target, options) in &face_specs {
            let Some(expected) = row[face].as_str() else {
                mismatches.push(format!("{key}/{face}: ruby lane returned a non-string"));
                continue;
            };
            let path = if *target == "schema" {
                schema_path.as_str()
            } else {
                ""
            };
            let actual =
                match expressir::formatter::format_flavored(wire, path, &options.to_string()) {
                    Ok(text) => text,
                    Err(message) => {
                        mismatches.push(format!("{key}/{face}: rust lane failed: {message}"));
                        continue;
                    }
                };
            compared += 1;
            if actual != expected {
                mismatches.push(format!(
                    "{key}/{face}: rust {} bytes / ruby {} bytes — first diff at {}",
                    actual.len(),
                    expected.len(),
                    first_diff(&actual, expected)
                ));
            }
        }
    }

    if !mismatches.is_empty() {
        panic!(
            "flavor mismatches ({} of {}):\n{}",
            mismatches.len(),
            compared,
            mismatches.join("\n")
        );
    }
    eprintln!("flavor parity: {compared} compared, 0 mismatches");
}
