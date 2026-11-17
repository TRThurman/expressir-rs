//! Rust-vs-Ruby interpreter parity (expressir#639, slice 2).
//!
//! The schema wire comes from the Rust lane over the baked fixture;
//! the Ruby lane hydrates the ExpFile from that same wire, builds
//! `Expressir::Eval::Environment(schema: …)` and calls each case
//! through `Interpreter#call`; the Rust lane replays the same calls
//! through `eval_interpreter::call_json`. Results — including
//! `Eval::Error` messages — must match value for value. Skipped
//! without `EXPRESSIR_DIR`.

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
  when NilClass then { "nil" => true }
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
model = Expressir::Model::ExpFile.from_hash(request["schema_file"])
model.wire_parents
schema = model.schemas.first
env = Expressir::Eval::Environment.new(schema: schema)
interpreter = env.interpreter
results = {}
errors = {}
request["cases"].each do |c|
  begin
    arguments = c["arguments"].map { |a| decode.call(a) }
    results[c["id"]] = encode.call(interpreter.call(c["name"], arguments))
  rescue Expressir::Eval::Error => e
    results[c["id"]] = { "error" => e.message }
  rescue StandardError => e
    errors[c["id"]] = "#{e.class}: #{e.message}"
  end
end
puts JSON.generate({ "results" => results, "errors" => errors })
"##;

fn expressir_dir() -> Option<PathBuf> {
    std::env::var_os("EXPRESSIR_DIR").map(PathBuf::from)
}

type LaneMaps = Result<(Map<String, Value>, Map<String, Value>), String>;

fn ruby_results(
    expressir_dir: &Path,
    schema_file: &Value,
    cases: &[(String, String, Vec<Value>)],
) -> LaneMaps {
    let request = json!({
        "schema_file": schema_file,
        "cases": cases
            .iter()
            .map(|(id, name, arguments)| {
                json!({ "id": id, "name": name, "arguments": arguments })
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

fn arg_list(values: &[Value]) -> Vec<Value> {
    values.to_vec()
}

#[test]
fn eval_interpreter_parity() {
    let Some(expressir_dir) = expressir_dir() else {
        eprintln!("EXPRESSIR_DIR unset — skipping the interpreter parity run");
        return;
    };
    let source = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eval_interp_gate.exp"),
    )
    .expect("gate fixture");
    let tree = expressir_rs::ParsedTree::parse(&source).expect("rs lane parses the fixture");
    let wire = tree
        .to_model_json("eval_interp_gate.exp")
        .expect("rs lane emits the wire");
    let schema = wire
        .get("schemas")
        .and_then(Value::as_array)
        .and_then(|s| s.first())
        .cloned()
        .expect("schema node");

    let cases: Vec<(String, String, Vec<Value>)> = vec![
        ("add".into(), "add".into(), arg_list(&[json!(3), json!(4)])),
        (
            "add_neg".into(),
            "add".into(),
            arg_list(&[json!(-3), json!(4)]),
        ),
        ("sum_to_5".into(), "sum_to".into(), arg_list(&[json!(5)])),
        ("sum_to_0".into(), "sum_to".into(), arg_list(&[json!(0)])),
        ("sum_down".into(), "sum_down".into(), arg_list(&[json!(4)])),
        (
            "classify_1".into(),
            "classify".into(),
            arg_list(&[json!(1)]),
        ),
        (
            "classify_2".into(),
            "classify".into(),
            arg_list(&[json!(2)]),
        ),
        (
            "classify_3".into(),
            "classify".into(),
            arg_list(&[json!(3)]),
        ),
        (
            "classify_other".into(),
            "classify".into(),
            arg_list(&[json!(9)]),
        ),
        ("pick_then".into(), "pick".into(), arg_list(&[json!(7)])),
        ("pick_else".into(), "pick".into(), arg_list(&[json!(2)])),
        (
            "skip_odds".into(),
            "skip_odds".into(),
            arg_list(&[json!(6)]),
        ),
        (
            "first_even".into(),
            "first_even".into(),
            arg_list(&[json!(5)]),
        ),
        (
            "first_even_none".into(),
            "first_even".into(),
            arg_list(&[json!(1)]),
        ),
        ("grow".into(), "grow".into(), arg_list(&[json!(4)])),
        ("grow_empty".into(), "grow".into(), arg_list(&[json!(0)])),
        ("patch".into(), "patch".into(), arg_list(&[])),
        ("chain".into(), "chain".into(), arg_list(&[json!(3)])),
        ("fact_6".into(), "fact".into(), arg_list(&[json!(6)])),
        (
            "with_local_expr".into(),
            "with_local_expr".into(),
            arg_list(&[json!(2)]),
        ),
        ("color_same".into(), "color_same".into(), arg_list(&[])),
        ("color_value".into(), "color_value".into(), arg_list(&[])),
        (
            "while_sum".into(),
            "while_sum".into(),
            arg_list(&[json!(4)]),
        ),
        (
            "while_sum_zero".into(),
            "while_sum".into(),
            arg_list(&[json!(0)]),
        ),
        (
            "until_sum".into(),
            "until_sum".into(),
            arg_list(&[json!(4)]),
        ),
        (
            "alias_err".into(),
            "alias_err".into(),
            arg_list(&[json!(1)]),
        ),
        (
            "zero_step".into(),
            "zero_step".into(),
            arg_list(&[json!(1)]),
        ),
        (
            "unknown_builtin".into(),
            "unknown_builtin".into(),
            arg_list(&[json!(1)]),
        ),
        (
            "ends_without".into(),
            "ends_without".into(),
            arg_list(&[json!(2)]),
        ),
        (
            "unbound_body".into(),
            "unbound_body".into(),
            arg_list(&[json!(1)]),
        ),
    ];

    let (ruby, harness_errors) =
        ruby_results(&expressir_dir, &wire, &cases).expect("ruby interpreter lane");
    assert!(
        harness_errors.is_empty(),
        "ruby harness raised: {harness_errors:?}"
    );

    let schema_json = schema.to_string();
    let mut mismatches: Vec<String> = Vec::new();
    for (id, name, arguments) in &cases {
        let rust = match expressir_rs::eval_interpreter::call_json(
            &schema_json,
            name,
            &serde_json::to_string(arguments).expect("arguments json"),
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
            "interpreter parity mismatches ({} of {}):\n{}",
            mismatches.len(),
            cases.len(),
            mismatches.join("\n")
        );
    }
}
