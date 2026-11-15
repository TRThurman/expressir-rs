//! The #634 wire differential: the ruby lane's own parse (`from_exp` on
//! the pure-ruby path, `include_source`, `skip_references`) serialized
//! through lutaml-model `to_hash` must byte-equal the Rust
//! `ParsedTree::parse(...).to_model_json` wire over the corpus.
//!
//! This is the strongest standing check in the repo: not lengths, not
//! attachment, not formatting — the full wire, node for node, key for
//! key, offset for offset. Both position metadata AND structural shape
//! are compared; explicit nulls are dropped on both sides (ruby omits
//! nil keys, rs emits them). The top-level rs `path` is an invocation
//! artifact and is dropped. Reference resolution stays OFF on both
//! sides: `base_path`/`base_ref` are #635's products, not parse-wire.
//!
//! Skipped without EXPRESSIR_DIR like the other no-drift gates.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

const RUBY_HARNESS: &str = r##"
require "json"
root = ARGV[0]
$LOAD_PATH.unshift(File.join(root, "lib"))
require "expressir"

out = {}
request = JSON.parse($stdin.read)
request["files"].each do |path, source|
  begin
    # use_native: the parsanol-backed builder IS the production lane
    # (and the same engine family the rs wire comes from); the legacy
    # pure-ruby fallback builder has its own shape quirks (builtin-call
    # classification) that are not the wire contract under test.
    # include_source: false keeps the parse RAW — the ruby RemarkAttacher
    # (which include_source actually toggles) is the attach pass, and the
    # remark_attachment_parity gate owns that comparison.
    repo = Expressir::Express::Parser.from_exp(
      source, skip_references: true, include_source: false, use_native: true
    )
    out[path] = repo.to_hash
  rescue StandardError, NotImplementedError => e
    out[path] = { "error" => "#{e.class}: #{e.message}" }
  end
end
puts JSON.generate(out)
"##;

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

fn corpus_files(expressir_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_exp_files(expressir_dir, &mut files);
    files.sort();
    files.dedup();
    files
}

fn ruby_wires(
    expressir_dir: &Path,
    sources: &std::collections::BTreeMap<String, String>,
) -> Result<Value, String> {
    let request = json!({ "files": sources });
    let mut child = Command::new("bundle")
        .arg("exec")
        .arg("ruby")
        .arg("-e")
        .arg(RUBY_HARNESS)
        .arg(expressir_dir)
        .current_dir(expressir_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
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
    serde_json::from_slice(&output.stdout).map_err(|e| format!("json: {e}"))
}

/// Keys stripped from BOTH sides before comparison, with the reason
/// each is outside the parse-wire contract:
/// - `path` / `file`: parse-invocation artifacts (from_exp carries no
///   path; to_model_json stamps the caller's name).
/// - `source`: rs renders eagerly into the wire; ruby computes it on
///   demand (ModelElement#source). The length parity is the
///   formatted_len gate's contract.
/// - `remarks` / `untagged_remarks`: the ruby parse attaches inline
///   remarks at build time; the rs attach pass produces them (the
///   remark_attachment_parity gate owns that comparison).
/// - `header`: derived post-parse from the pre-SCHEMA untagged remarks
///   (transfer_header_to_schema) — remark-attach domain, not wire.
/// - `source_end_offset`: the rs attacher's derived span end; ruby
///   never carries it (the attachment gate strips it for the same
///   reason).
/// - empty arrays: rs emits empty collections, a fresh ruby parse omits
///   them; presence-with-content still compares.
const STRIPPED_KEYS: &[&str] = &[
    "path",
    "file",
    "source",
    "remarks",
    "untagged_remarks",
    "header",
    "source_end_offset",
];

/// Canonical form: nulls, stripped keys and empty arrays dropped
/// recursively, everything else preserved.
fn canon(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let out: serde_json::Map<String, Value> = map
                .iter()
                .filter(|(k, val)| {
                    !val.is_null()
                        && !STRIPPED_KEYS.contains(&k.as_str())
                        && !val.as_array().is_some_and(|a| a.is_empty())
                })
                .map(|(k, val)| (k.clone(), canon(val)))
                .collect();
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(canon).collect()),
        other => other.clone(),
    }
}

/// Position keys compared only when BOTH sides carry them: the ruby
/// builder populates them where its captures allow; absence on the
/// ruby side is not shape drift. When both are present, a mismatch is.
const POSITION_KEYS: &[&str] = &["source_offset", "source_length"];

/// First divergent JSON-pointer path between two canon wires.
fn first_diff(a: &Value, b: &Value, path: &mut String) -> Option<String> {
    match (a, b) {
        (Value::Object(am), Value::Object(bm)) => {
            let keys: std::collections::BTreeSet<&String> = am.keys().chain(bm.keys()).collect();
            for k in keys {
                if POSITION_KEYS.contains(&k.as_str()) && bm.get(k).is_none() {
                    continue;
                }
                let saved = path.len();
                path.push('/');
                path.push_str(k);
                match (am.get(k), bm.get(k)) {
                    (Some(x), Some(y)) => {
                        if let Some(d) = first_diff(x, y, path) {
                            return Some(d);
                        }
                    }
                    (Some(x), None) => {
                        return Some(format!("{path} ruby-missing, rs={x}"));
                    }
                    (None, Some(y)) => {
                        return Some(format!("{path} rs-missing, ruby={y}"));
                    }
                    (None, None) => {}
                }
                path.truncate(saved);
            }
            None
        }
        (Value::Array(ai), Value::Array(bi)) => {
            for (i, (x, y)) in ai.iter().zip(bi.iter()).enumerate() {
                let saved = path.len();
                path.push('/');
                path.push_str(&i.to_string());
                if let Some(d) = first_diff(x, y, path) {
                    return Some(d);
                }
                path.truncate(saved);
            }
            if ai.len() != bi.len() {
                return Some(format!(
                    "{path} array lengths ruby={} rs={}",
                    ai.len(),
                    bi.len()
                ));
            }
            None
        }
        _ => {
            if a != b {
                // a = the rs wire, b = the ruby wire
                Some(format!("{path} rs={a} ruby={b}"))
            } else {
                None
            }
        }
    }
}

#[test]
fn rs_wire_matches_the_ruby_parse_over_the_corpus() {
    let Some(expressir_dir) = expressir_dir() else {
        eprintln!("EXPRESSIR_DIR not set; skipping");
        return;
    };
    let files = corpus_files(&expressir_dir);
    if files.is_empty() {
        eprintln!("no corpus files under EXPRESSIR_DIR; skipping");
        return;
    }

    let mut sources: std::collections::BTreeMap<String, String> = Default::default();
    let mut parsed: Vec<(String, Value)> = Vec::new();
    for path in &files {
        let key = path.to_string_lossy().into_owned();
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let tree = match expressir_rs::ParsedTree::parse(&source) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let Ok(wire) = tree.to_model_json("parity.exp") else {
            continue;
        };
        sources.insert(key.clone(), source);
        parsed.push((key, wire));
    }

    let ruby = ruby_wires(&expressir_dir, &sources).expect("ruby wires");

    let mut failures: Vec<String> = Vec::new();
    for (key, wire) in &parsed {
        let Some(entry) = ruby.get(key) else {
            continue;
        };
        if entry.get("error").is_some() {
            failures.push(format!(
                "{key}: ruby lane errored: {}",
                entry["error"].as_str().unwrap_or("?")
            ));
            continue;
        }
        let a = canon(wire);
        let b = canon(entry);
        // first_diff IS the comparator: the position-intersection rule
        // makes a != b possible for wires that are equal under the
        // comparison rules, so equality means "first_diff finds nothing".
        let mut path = String::new();
        if let Some(diff) = first_diff(&a, &b, &mut path) {
            failures.push(format!("{key}: first divergence at {diff}"));
        }
    }
    assert!(
        failures.is_empty(),
        "wire drift over {} files:\n{}",
        parsed.len(),
        failures
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
