//! Rust-vs-Ruby interop parity (expressir#641): OWL 2 Turtle,
//! JSON Schema 2020-12 and the interface GraphViz emitter. The Ruby
//! lane runs `Interop::Owl.export` / `Interop::JsonSchema.export` /
//! `Express::InterfaceDot#write` over hydrated wires; the rs lane
//! replays `interop::{owl_json, json_schema_json, interface_dot}`.
//! Skipped without EXPRESSIR_DIR.

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
    schema = model.schemas.first
    row = {}
    if schema
      row["owl"] = Expressir::Interop::Owl.export(schema, base_iri: "https://gate.example/x#")
      row["json_schema"] = JSON.generate(Expressir::Interop::JsonSchema.export(schema))
    end
    if model.schemas.length > 1
      repository = Expressir::Model::Repository.new(schemas: model.schemas)
      dot = Expressir::Express::InterfaceDot.new(model.schemas.first, repository, iface: :both, depth: nil, prune: []).write
      row["dot"] = dot
    end
    out[path] = row
  rescue StandardError => e
    errors[path] = "#{e.class}: #{e.message}"
  end
end
puts JSON.generate({ "texts" => out, "errors" => errors, "version" => Expressir::VERSION })
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

type LaneMaps = Result<(Map<String, Value>, Map<String, Value>, String), String>;

fn ruby_lanes(expressir_dir: &Path, wires: &Map<String, Value>) -> LaneMaps {
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
        parsed["version"].as_str().unwrap_or_default().to_string(),
    ))
}

fn repository_wire(wire: &Value) -> Value {
    // InterfaceDot takes a repository; the ExpFile wire's schemas
    // list is the inventory.
    json!({ "schemas": wire.get("schemas").cloned().unwrap_or(Value::Array(Vec::new())) })
}

#[test]
fn interop_parity() {
    // The big corpus trees recurse deeply — run on a fat stack.
    let handle = std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(run_parity)
        .expect("fat-stack thread");
    handle.join().expect("parity run");
}

fn run_parity() {
    let Some(expressir_dir) = expressir_dir() else {
        eprintln!("EXPRESSIR_DIR unset — skipping the interop parity run");
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
        // InterfaceDot reads the interfaces as resolved schema names —
        // resolve the wire first (the ruby lane's InterfaceDot reads
        // iface.schema strings left by the native parse).
        expressir::checker::resolve_references(&mut tree_wire);
        wires.insert(key, tree_wire);
    }

    let (ruby, errors, version) = ruby_lanes(&expressir_dir, &wires).expect("ruby interop lane");
    assert!(errors.is_empty(), "ruby harness raised: {errors:?}");

    let mut compared = 0usize;
    let mut mismatches: Vec<String> = Vec::new();
    for (key, wire) in &wires {
        let Some(row) = ruby.get(key) else {
            mismatches.push(format!("{key}: missing from the ruby lane"));
            continue;
        };
        let schema = wire["schemas"]
            .as_array()
            .and_then(|schemas| schemas.first())
            .cloned()
            .unwrap_or(Value::Null);

        if let Some(expected) = row["owl"].as_str() {
            let actual = match expressir::interop::owl_json(
                &schema.to_string(),
                "https://gate.example/x#",
            ) {
                Ok(text) => text,
                Err(message) => format!("ERROR {message}"),
            };
            compared += 1;
            if actual != expected {
                mismatches.push(format!(
                    "{key}/owl: rust {} bytes / ruby {} bytes — first diff at {}",
                    actual.len(),
                    expected.len(),
                    actual
                        .as_bytes()
                        .iter()
                        .zip(expected.as_bytes().iter())
                        .position(|(a, b)| a != b)
                        .unwrap_or(actual.len().min(expected.len()))
                ));
            }
        }

        if let Some(expected) = row["json_schema"].as_str() {
            let actual = match expressir::interop::json_schema_json(&schema.to_string()) {
                Ok(text) => text,
                Err(message) => format!("ERROR {message}"),
            };
            compared += 1;
            let actual_value: Value = serde_json::from_str(&actual).unwrap_or(Value::Null);
            let expected_value: Value = serde_json::from_str(expected).unwrap_or(Value::Null);
            if actual_value != expected_value {
                mismatches.push(format!("{key}/json_schema: structures differ"));
            }
        }

        if let Some(expected) = row["dot"].as_str() {
            let repository = repository_wire(wire);
            let options = expressir::interop::DotOptions {
                iface: "both",
                uf_color: "blue",
                rf_color: "green",
                depth: None,
                prune: &[],
                version: &version,
            };
            let actual = expressir::interop::interface_dot(&repository, 0, &options);
            compared += 1;
            if actual != expected {
                mismatches.push(format!(
                    "{key}/dot: rust {} bytes / ruby {} bytes — first diff at {}",
                    actual.len(),
                    expected.len(),
                    actual
                        .as_bytes()
                        .iter()
                        .zip(expected.as_bytes().iter())
                        .position(|(a, b)| a != b)
                        .unwrap_or(actual.len().min(expected.len()))
                ));
            }
        }
    }

    if !mismatches.is_empty() {
        panic!(
            "interop mismatches ({} of {}):\n{}",
            mismatches.len(),
            compared,
            mismatches.join("\n")
        );
    }
    eprintln!("interop parity: {compared} compared, 0 mismatches");
}
