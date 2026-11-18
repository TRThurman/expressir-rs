//! Rust-vs-Ruby schema-compare parity (expressir#638).
//!
//! Both lanes diff two schema wires (`SchemaCompare.compare`) and
//! apply the deltas (`SchemaPatch.apply`); the delta sequences must
//! match op-for-op/path-for-path (nodes compared under the signature
//! normalization) and the patched schemas must equal the revised
//! schema's signature on both lanes. Skipped without `EXPRESSIR_DIR`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

const RUBY_HARNESS: &str = r####"
require "json"
root = ARGV[0]
$LOAD_PATH.unshift(File.join(root, "lib"))
require "expressir"

SC = Expressir::Express::SchemaCompare
SP = Expressir::Express::SchemaPatch

request = JSON.parse($stdin.read)
results = {}
errors = {}
request["cases"].each do |c|
  begin
    base = Expressir::Model::Declarations::Schema.from_hash(c["base"])
    revised = Expressir::Model::Declarations::Schema.from_hash(c["revised"])
    deltas = SC.compare(base, revised)
    encoded = deltas.map do |d|
      {
        "op" => d.op.to_s,
        "path" => d.path,
        "node" => d.node&.to_hash,
      }
    end
    patched = SP.apply(base, deltas)
    results[c["id"]] = {
      "deltas" => encoded,
      "patched_signature" => SC.strip(patched.to_hash),
      "revised_signature" => SC.strip(revised.to_hash),
    }
  rescue StandardError => e
    errors[c["id"]] = "#{e.class}: #{e.message}"
  end
end
puts JSON.generate({ "results" => results, "errors" => errors })
"####;

fn expressir_dir() -> Option<PathBuf> {
    std::env::var_os("EXPRESSIR_DIR").map(PathBuf::from)
}

fn wire_of(path: &Path, name: &str) -> Value {
    let source = std::fs::read_to_string(path).expect("fixture readable");
    let tree = expressir::ParsedTree::parse(&source)
        .unwrap_or_else(|e| panic!("rs lane parses {name}: {e:?}"));
    let wire = tree
        .to_model_json(name)
        .unwrap_or_else(|e| panic!("rs wire for {name}: {e:?}"));
    wire["schemas"]
        .as_array()
        .and_then(|schemas| schemas.first())
        .cloned()
        .expect("schema node")
}

fn ruby_lanes(expressir_dir: &Path, cases: &[(String, Value, Value)]) -> (Value, Value) {
    let request = json!({
        "cases": cases
            .iter()
            .map(|(id, base, revised)| {
                json!({ "id": id, "base": base, "revised": revised })
            })
            .collect::<Vec<_>>(),
    });
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
        .expect("bundle/ruby");
    use std::io::Write;
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(request.to_string().as_bytes())
        .expect("write");
    let output = child.wait_with_output().expect("wait");
    if !output.status.success() {
        panic!(
            "ruby harness failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let parsed: Value = serde_json::from_slice(&output.stdout).expect("json");
    (parsed["results"].clone(), parsed["errors"].clone())
}

#[test]
fn schema_compare_parity() {
    let Some(expressir_dir) = expressir_dir() else {
        eprintln!("EXPRESSIR_DIR unset — skipping the schema compare parity run");
        return;
    };
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let base = wire_of(
        &fixtures.join("compare_gate_base.exp"),
        "compare_gate_base.exp",
    );
    let revised = wire_of(
        &fixtures.join("compare_gate_revised.exp"),
        "compare_gate_revised.exp",
    );

    let cases: Vec<(String, Value, Value)> = vec![("gate".into(), base.clone(), revised.clone())];

    let (ruby, errors) = ruby_lanes(&expressir_dir, &cases);
    assert!(
        errors
            .as_object()
            .unwrap_or(&serde_json::Map::new())
            .is_empty(),
        "ruby harness raised: {errors:?}"
    );

    let row = &ruby["gate"];
    let rs_deltas_raw = expressir::schema_compare::compare(&base, &revised);

    let ruby_deltas = &row["deltas"];
    // Node payloads: ruby to_hash vs the rs wire — the wire gate's
    // normalization (drop empty arrays + explicit nulls) applies.
    let normalize = |value: &Value| -> Value {
        const DROPPED: &[&str] = &[
            "source",
            "source_offset",
            "source_end_offset",
            "source_length",
            "include_source",
            "ref",
            "base_path",
            "file",
        ];
        fn walk(value: &Value) -> Value {
            match value {
                Value::Object(map) => {
                    let out: serde_json::Map<String, Value> = map
                        .iter()
                        .filter(|(k, _)| !DROPPED.contains(&k.as_str()))
                        .filter(|(_, v)| !v.is_null())
                        .filter(|(_, v)| !(v.is_array() && v.as_array().unwrap().is_empty()))
                        .map(|(k, v)| (k.clone(), walk(v)))
                        .collect();
                    Value::Object(out)
                }
                Value::Array(items) => Value::Array(items.iter().map(walk).collect()),
                other => other.clone(),
            }
        }
        walk(value)
    };
    let rs_norm: Vec<Value> = rs_deltas_raw
        .iter()
        .map(|delta| {
            let mut delta = delta.clone();
            if delta["node"].is_null() {
                delta
            } else {
                delta["node"] = normalize(&delta["node"]);
                delta
            }
        })
        .collect();
    let ruby_norm: Vec<Value> = ruby_deltas
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|mut delta| {
            if delta["node"].is_null() {
                delta
            } else {
                delta["node"] = normalize(&delta["node"]);
                delta
            }
        })
        .collect();

    let mut mismatches: Vec<String> = Vec::new();
    if rs_norm != ruby_norm {
        mismatches.push(format!(
            "deltas:\n  rust ={}\n  ruby ={}",
            serde_json::to_string(&rs_norm).unwrap_or_default(),
            serde_json::to_string(&ruby_norm).unwrap_or_default()
        ));
    }

    // Patch: both lanes' patched schemas must equal the revised
    // signature (the ruby PATCHED signature, normalized, is the
    // oracle for the wire shape too).
    let rs_patched = expressir::schema_compare::patch(&base, &rs_deltas_raw);
    let patched_normalized = normalize(&row["patched_signature"]);
    let revised_normalized = normalize(&row["revised_signature"]);
    if normalize(&rs_patched) != patched_normalized {
        mismatches.push("rs patched != ruby patched signature".to_string());
    }
    if patched_normalized != revised_normalized {
        mismatches.push("ruby patched != ruby revised signature (lane sanity)".to_string());
    }
    if normalize(&rs_patched) != revised_normalized {
        mismatches.push("rs patched != rs-side revised signature".to_string());
    }

    assert!(
        mismatches.is_empty(),
        "schema compare mismatches:\n{}",
        mismatches.join("\n")
    );
    eprintln!(
        "schema compare parity: {} deltas compared, patch == revised",
        rs_norm.len()
    );
}
