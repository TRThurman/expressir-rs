//! Rust-vs-Ruby PrettyFormatter parity (expressir#637).
//!
//! The Ruby lane parses each corpus file on the native lane, emits
//! the wire, hydrates the ExpFile back and formats it with
//! `Expressir::Express::PrettyFormatter` under pinned options; the
//! Rust lane replays the same wire through
//! `formatter::pretty_format`. Byte-equal output, including a
//! provenance-on case with a pinned tool name/version. Skipped
//! without `EXPRESSIR_DIR`.

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
    options = request["options"].map { |k, v| [k.to_sym, v] }.to_h
    formatter = Expressir::Express::PrettyFormatter.new(options)
    out[path] = formatter.format(model)
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

fn ruby_texts(expressir_dir: &Path, wires: &Map<String, Value>, options: &Value) -> LaneMaps {
    let request = json!({ "files": wires, "options": options });
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

/// The big corpus trees recurse deeply — run the lane on a fat stack.
fn run_parity_on_fat_stack(options: Value) {
    let handle = std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(move || run_parity(&options))
        .expect("fat-stack thread");
    handle.join().expect("parity run");
}

fn run_parity(options: &Value) {
    let Some(expressir_dir) = expressir_dir() else {
        eprintln!("EXPRESSIR_DIR unset — skipping the pretty formatter parity run");
        return;
    };
    let mut files = Vec::new();
    collect_exp_files(&expressir_dir, &mut files);
    files.sort();
    files.dedup();
    assert!(!files.is_empty(), "no .exp fixtures found");

    let mut wires: Map<String, Value> = Map::new();
    let mut rust: Map<String, Value> = Map::new();
    for path in &files {
        let key = path.to_string_lossy().into_owned();
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(tree) = expressir::ParsedTree::parse(&source) else {
            eprintln!("skip (rust lane cannot parse): {key}");
            continue;
        };
        let Ok(wire) = tree.to_model_json(&key) else {
            continue;
        };
        match expressir::formatter::pretty_format(&wire, "", &options.to_string()) {
            Ok(text) => {
                rust.insert(key.clone(), Value::String(text));
                wires.insert(key, wire);
            }
            Err(message) => eprintln!("skip (rust lane cannot format): {key}: {message}"),
        }
    }

    let (ruby, errors) = ruby_texts(&expressir_dir, &wires, options).expect("ruby pretty lane");
    assert!(errors.is_empty(), "ruby harness raised: {errors:?}");

    let mut mismatches: Vec<String> = Vec::new();
    for (key, expected) in &ruby {
        let Some(expected_text) = expected.as_str() else {
            mismatches.push(format!("{key}: ruby lane returned a non-string"));
            continue;
        };
        let Some(actual) = rust.get(key) else {
            mismatches.push(format!("{key}: missing from the rust lane"));
            continue;
        };
        let actual_text = actual.as_str().unwrap_or_default();
        if actual_text != expected_text {
            mismatches.push(format!(
                "{key}: rust {} bytes / ruby {} bytes — first diff at {}",
                actual_text.len(),
                expected_text.len(),
                {
                    let a = actual_text.as_bytes();
                    let b = expected_text.as_bytes();
                    a.iter()
                        .zip(b.iter())
                        .position(|(x, y)| x != y)
                        .unwrap_or(a.len().min(b.len()))
                }
            ));
        }
    }

    if !mismatches.is_empty() {
        panic!(
            "pretty formatter mismatches ({} of {}):\n{}",
            mismatches.len(),
            ruby.len(),
            mismatches.join("\n")
        );
    }
    eprintln!(
        "pretty formatter parity: {} compared, 0 mismatches",
        ruby.len()
    );
}

#[test]
fn pretty_formatter_parity_no_provenance() {
    run_parity_on_fat_stack(json!({
        "indent": 4,
        "provenance": false,
        "no_remarks": false
    }));
}

#[test]
fn pretty_formatter_parity_provenance() {
    run_parity_on_fat_stack(json!({
        "indent": 2,
        "provenance": true,
        "provenance_name": "Expressir",
        "provenance_version": "0.0.0-gate",
        "no_remarks": false
    }));
}
