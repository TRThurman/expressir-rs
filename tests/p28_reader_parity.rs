//! Rust-vs-Ruby P28 reader parity (expressir#640): `P28::Uos.parse`
//! and `P28::Document.parse` — the clause 9 reader faces. The Ruby
//! lane runs moxml; the rs lane runs the rs reader. Populations are
//! compared under the gate param encoding; express elements and the
//! error messages must match exactly. Skipped without EXPRESSIR_DIR.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Map, Value};

const RUBY_HARNESS: &str = r####"
require "json"
root = ARGV[0]
$LOAD_PATH.unshift(File.join(root, "lib"))
require "expressir"

param = lambda do |raw|
  case raw
  when Symbol then raw == :unset ? "unset" : "derived"
  when Expressir::P21::Enumeration then { "enum" => raw.name }
  when Expressir::P21::Binary then { "binary" => raw.hex }
  when Expressir::P21::TypedValue
    { "typed" => { "keyword" => raw.keyword, "params" => [param.call(raw.value)] } }
  when Array then { "list" => raw.map { |item| param.call(item) } }
  when Integer, Float then { "number" => raw.to_s }
  when TrueClass, FalseClass then { "bool" => raw }
  else raw.to_s
  end
end

population = lambda do |p|
  {
    "instances" => p.instances.map do |id, record|
      records = record.is_a?(Expressir::P21::ComplexRecord) ? record.records : [record]
      {
        "id" => id.delete_prefix("#").to_i,
        "records" => records.map do |r|
          { "keyword" => r.keyword, "params" => r.params.map { |x| param.call(x) } }
        end,
      }
    end,
  }
end

request = JSON.parse($stdin.read)
results = {}
errors = {}
request["cases"].each do |c|
  begin
    schema = Expressir::Model::Declarations::Schema.from_hash(c["schema"])
    schema.wire_parents
    row = {}
    if c["uos"]
      row["uos"] = population.call(Expressir::P28::Uos.parse(c["uos"], schema))
    end
    if c["document"]
      parsed = Expressir::P28::Document.parse(c["document"])
      row["document"] = {
        "express" => parsed[:express].map do |e|
          { "id" => e.id, "schema_name" => e.schema_name, "source" => e.source }
        end,
        "units" => parsed[:units].to_h { |id, p| [id, population.call(p)] },
      }
    end
    results[c["id"]] = row
  rescue Expressir::Error => e
    results[c["id"]] = { "error" => e.message }
  rescue StandardError => e
    errors[c["id"]] = "#{e.class}: #{e.message}"
  end
end
puts JSON.generate({ "results" => results, "errors" => errors })
"####;

fn expressir_dir() -> Option<PathBuf> {
    std::env::var_os("EXPRESSIR_DIR").map(PathBuf::from)
}

type LaneMaps = Result<(Map<String, Value>, Map<String, Value>), String>;

fn ruby_lanes(
    expressir_dir: &Path,
    cases: &[(String, Value, Option<String>, Option<String>)],
) -> LaneMaps {
    let request = json!({
        "cases": cases
            .iter()
            .map(|(id, schema, uos, document)| {
                let mut c = json!({ "id": id, "schema": schema });
                if let Some(uos) = uos {
                    c["uos"] = json!(uos);
                }
                if let Some(document) = document {
                    c["document"] = json!(document);
                }
                c
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
        parsed["results"].as_object().cloned().unwrap_or_default(),
        parsed["errors"].as_object().cloned().unwrap_or_default(),
    ))
}

fn schema_wire_of(path: &Path, name: &str) -> Value {
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

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).expect("fixture readable")
}

#[test]
fn p28_reader_parity() {
    let Some(expressir_dir) = expressir_dir() else {
        eprintln!("EXPRESSIR_DIR unset — skipping the P28 reader parity run");
        return;
    };
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let schema = schema_wire_of(&fixtures.join("p28_reader_gate.exp"), "p28_reader_gate.exp");
    let uos = read(&fixtures.join("p28_reader_gate.uos.xml"));
    let document = read(&fixtures.join("p28_reader_gate.doc.xml"));

    let bad_root = "<wrong_root/>";
    let unknown_uos = format!(
        "<iso_10303_28><express id=\"x1\" schema_name=\"p28_reader_gate\">{}</express><uos schema=\"ghost\"/></iso_10303_28>",
        read(&fixtures.join("p28_reader_gate.exp")).replace('<', "&lt;").replace('>', "&gt;")
    );

    let cases: Vec<(String, Value, Option<String>, Option<String>)> = vec![
        ("uos_values".into(), schema.clone(), Some(uos.clone()), None),
        ("document".into(), schema.clone(), None, Some(document)),
        (
            "bad_uos_root".into(),
            schema.clone(),
            Some(bad_root.to_string()),
            None,
        ),
        (
            "unknown_express".into(),
            schema.clone(),
            None,
            Some(unknown_uos),
        ),
    ];

    let (ruby, harness_errors) = ruby_lanes(&expressir_dir, &cases).expect("ruby P28 reader lane");
    assert!(
        harness_errors.is_empty(),
        "ruby harness raised: {harness_errors:?}"
    );

    let mut mismatches: Vec<String> = Vec::new();
    for (id, schema, uos, document) in &cases {
        let expected = ruby.get(id).expect("ruby lane result");
        if let Some(message) = expected.get("error").and_then(Value::as_str) {
            let actual_error = match (uos, document) {
                (Some(uos), _) => {
                    expressir::p28::uos_parse_json(uos, &schema.to_string()).unwrap_err()
                }
                (_, Some(document)) => expressir::p28::document_parse_json(document).unwrap_err(),
                _ => continue,
            };
            if actual_error != message {
                mismatches.push(format!(
                    "{id}: rust error '{actual_error}' / ruby error '{message}'"
                ));
            }
            continue;
        }
        let (expected, actual): (Value, Value) = match (uos, document) {
            (Some(uos), _) => (
                expected.get("uos").cloned().unwrap_or(Value::Null),
                match expressir::p28::uos_parse_json(uos, &schema.to_string()) {
                    Ok(text) => serde_json::from_str(&text).expect("population json"),
                    Err(message) => json!({ "error": message }),
                },
            ),
            (_, Some(document)) => (
                expected.clone(),
                match expressir::p28::document_parse_json(document) {
                    Ok(text) => serde_json::from_str(&text).expect("document json"),
                    Err(message) => json!({ "error": message }),
                },
            ),
            _ => continue,
        };
        if actual != expected {
            mismatches.push(format!(
                "{id}:\n  rust ={}\n  ruby ={}",
                serde_json::to_string(&actual).unwrap_or_default(),
                serde_json::to_string(&expected).unwrap_or_default()
            ));
        }
    }

    if !mismatches.is_empty() {
        panic!("P28 reader mismatches:\n{}", mismatches.join("\n"));
    }
    eprintln!("P28 reader parity: {} cases, 0 mismatches", cases.len());
}
