//! Rust-vs-Ruby evaluator parity (expressir#639, slice 1).
//!
//! The wire comes from the Rust lane (`ParsedTree::parse` →
//! `to_model_json`) over the baked gate fixture; the Ruby lane
//! hydrates each collected expression node from that same wire
//! (`klass.from_hash`) and evaluates it with `Expressir::Eval`
//! seeded from a shared variables map; the Rust lane replays the
//! same nodes through `eval::evaluate_json`. Results — including
//! `Eval::Error` messages — must match value for value. Skipped
//! without `EXPRESSIR_DIR` like the other differential gates.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Map, Value};

const RUBY_HARNESS: &str = r##"
require "json"
root = ARGV[0]
$LOAD_PATH.unshift(File.join(root, "lib"))
require "expressir"

V = Expressir::Eval::Values

decode = lambda do |w|
  case w
  when Hash
    return V::INDETERMINATE if w["indeterminate"]
    return V::Logical.new(value: w["logical"].downcase.to_sym) if w["logical"]
    return V::Enumeration.new(type: nil, item: w["enum"]) if w["enum"]
    return V::Binary.new(bits: w["binary"]) if w["binary"]
    if (agg = w["aggregate"])
      V::Aggregate.of(agg["kind"].to_sym, agg["elements"].map { |e| decode.call(e) })
    end
  else
    w
  end
end

encode = lambda do |v|
  case v
  when ->(x) { x.equal?(V::INDETERMINATE) } then { "indeterminate" => true }
  when V::Logical then { "logical" => v.to_s }
  when V::Enumeration then { "enum" => v.item }
  when V::Binary then { "binary" => v.bits }
  when V::Aggregate
    {
      "aggregate" => {
        "kind" => v.kind.to_s,
        "elements" => v.elements.map { |e| encode.call(e) },
      },
    }
  when Integer, Float, String then v
  else { "unsupported" => v.class.name }
  end
end

request = JSON.parse($stdin.read)
results = {}
errors = {}
request["cases"].each do |c|
  begin
    wire = c["wire"]
    klass = Object.const_get(wire["_class"])
    node = klass.from_hash(wire)
    env = Expressir::Eval::Environment.new(schema: nil)
    request["variables"].each do |name, value|
      env = env.define(name, decode.call(value))
    end
    results[c["id"]] = encode.call(Expressir::Eval::Expression.evaluate(node, env))
  rescue Expressir::Eval::Error => e
    results[c["id"]] = { "error" => e.message }
  rescue StandardError => e
    errors[c["id"]] = "#{e.class}: #{e.message}"
  end
end
puts JSON.generate({ "results" => results, "errors" => errors })
"##;

const EXPR_CLASSES: &[&str] = &[
    "Expressir::Model::Literals::Integer",
    "Expressir::Model::Literals::Real",
    "Expressir::Model::Literals::String",
    "Expressir::Model::Literals::Binary",
    "Expressir::Model::Literals::Logical",
    "Expressir::Model::Expressions::BinaryExpression",
    "Expressir::Model::Expressions::UnaryExpression",
    "Expressir::Model::Expressions::Interval",
    "Expressir::Model::Expressions::AggregateInitializer",
    "Expressir::Model::Expressions::QueryExpression",
    "Expressir::Model::Expressions::FunctionCall",
    "Expressir::Model::References::SimpleReference",
    "Expressir::Model::References::AttributeReference",
    "Expressir::Model::References::IndexReference",
    "Expressir::Model::References::GroupReference",
];

fn expressir_dir() -> Option<PathBuf> {
    std::env::var_os("EXPRESSIR_DIR").map(PathBuf::from)
}

fn collect_expressions(node: &Value, out: &mut Vec<(String, Value)>) {
    match node {
        Value::Object(map) => {
            let class = map.get("_class").and_then(Value::as_str).unwrap_or("");
            if EXPR_CLASSES.contains(&class) {
                let id = format!("e{}", out.len());
                out.push((id, node.clone()));
                return; // top-most wins: the parent evaluates its children
            }
            for value in map.values() {
                collect_expressions(value, out);
            }
        }
        Value::Array(items) => {
            for value in items {
                collect_expressions(value, out);
            }
        }
        _ => {}
    }
}

fn variables() -> Map<String, Value> {
    serde_json::from_value(json!({
        "x": 5,
        "y": 2.5,
        "neg": -7,
        "s": "hello",
        "b": { "binary": "1010" },
        "flag": { "logical": "TRUE" },
        "unk": { "logical": "UNKNOWN" },
        "col": { "enum": "red" },
        "green": { "enum": "green" },
        "items": { "aggregate": { "kind": "list", "elements": [1, 2, 3, 4] } },
        "empty_items": { "aggregate": { "kind": "list", "elements": [] } },
        "num_set": { "aggregate": { "kind": "set", "elements": [1, 2] } },
        "other_set": { "aggregate": { "kind": "set", "elements": [2, 3] } },
        "maybe": { "indeterminate": true },
        "strnum": "42.5"
    }))
    .expect("variables json")
}

type LaneMaps = Result<(Map<String, Value>, Map<String, Value>), String>;

fn ruby_results(expressir_dir: &Path, variables: &Value, cases: &[(String, Value)]) -> LaneMaps {
    let request = json!({
        "variables": variables,
        "cases": cases
            .iter()
            .map(|(id, wire)| json!({ "id": id, "wire": wire }))
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
    let results = parsed["results"].as_object().cloned().unwrap_or_default();
    let errors = parsed["errors"].as_object().cloned().unwrap_or_default();
    Ok((results, errors))
}

#[test]
fn eval_parity() {
    let Some(expressir_dir) = expressir_dir() else {
        eprintln!("EXPRESSIR_DIR unset — skipping the eval parity run");
        return;
    };
    let source = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eval_gate.exp"),
    )
    .expect("gate fixture");
    let tree = expressir_rs::ParsedTree::parse(&source).expect("rs lane parses the fixture");
    let wire = tree
        .to_model_json("eval_gate.exp")
        .expect("rs lane emits the wire");

    let mut cases: Vec<(String, Value)> = Vec::new();
    collect_expressions(&wire, &mut cases);
    assert!(cases.len() >= 70, "gate collected {}", cases.len());

    let vars = variables();
    let (ruby, harness_errors) =
        ruby_results(&expressir_dir, &Value::Object(vars.clone()), &cases).expect("ruby eval lane");
    assert!(
        harness_errors.is_empty(),
        "ruby harness raised: {harness_errors:?}"
    );

    let mut mismatches: Vec<String> = Vec::new();
    for (id, wire) in &cases {
        let rust = match expressir_rs::eval::evaluate_json(
            &wire.to_string(),
            &Value::Object(vars.clone()).to_string(),
        ) {
            Ok(result) => serde_json::from_str::<Value>(&result).expect("result json"),
            Err(message) => json!({ "error": message }),
        };
        let Some(expected) = ruby.get(id) else {
            mismatches.push(format!("{id}: missing from the ruby lane"));
            continue;
        };
        if &rust != expected {
            mismatches.push(format!("{id}: rust={rust} ruby={expected}"));
        }
    }

    if !mismatches.is_empty() {
        panic!(
            "eval parity mismatches ({} of {}):\n{}",
            mismatches.len(),
            cases.len(),
            mismatches.join("\n")
        );
    }
}
