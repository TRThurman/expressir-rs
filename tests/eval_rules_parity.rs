//! Rust-vs-Ruby WHERE-rule parity (expressir#639, slice 3).
//!
//! Both lanes check the SAME population against the SAME schema: the
//! Ruby lane parses the fixtures (native parse + `P21::Parser`),
//! applies the case mutations, emits the gate population encoding
//! and runs `Expressir::Eval::Rules.check`; the Rust lane replays
//! the emitted population through `eval_rules::check_json` over the
//! rs schema wire. Results — instance, rule id, status, message —
//! must match sequence-for-sequence. Skipped without `EXPRESSIR_DIR`.

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

decode_mutation = lambda do |m|
  case m
  when String
    m == "unset" ? :unset : m == "derived" ? :derived : m
  when Hash
    if m.key?("list")
      m["list"].map { |item| decode_mutation.call(item) }
    else
      m
    end
  else
    m
  end
end

request = JSON.parse($stdin.read)
results = {}
errors = {}
request["cases"].each do |c|
  begin
    model = Expressir::Model::ExpFile.from_hash(
      "_class" => "Expressir::Model::ExpFile",
      "schemas" => [c["schema_file"]]
    )
    model.wire_parents
    schema = model.schemas.first
    population = Expressir::P21::Parser.parse(c["p21"])
    c["mutations"].each do |m|
      target = population.instances.fetch("##{m["target"]}")
      target.params[m["index"]] = decode_mutation.call(m["value"])
    end
    encoded = {
      "instances" => population.instances.map do |id, record|
        records = record.is_a?(Expressir::P21::ComplexRecord) ? record.records : [record]
        {
          "id" => id.delete_prefix("#").to_i,
          "records" => records.map do |r|
            { "keyword" => r.keyword, "params" => r.params.map { |p| param.call(p) } }
          end,
        }
      end,
    }
    checked = Expressir::Eval::Rules.check(population, schema)
    results[c["id"]] = {
      "population" => encoded,
      "results" => checked.map do |r|
        {
          "instance" => r.instance,
          "rule_id" => r.rule_id,
          "status" => r.status.to_s,
          "message" => r.message,
        }
      end,
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

type LaneMaps = Result<(Map<String, Value>, Map<String, Value>), String>;

fn ruby_lanes(expressir_dir: &Path, cases: &[(String, Value, String, Value)]) -> LaneMaps {
    let request = json!({
        "cases": cases
            .iter()
            .map(|(id, schema, p21, mutations)| {
                json!({ "id": id, "schema_file": schema, "p21": p21, "mutations": mutations })
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

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).expect("fixture readable")
}

fn schema_wire_of(source: &str, name: &str) -> Value {
    let tree = expressir::ParsedTree::parse(source)
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

#[test]
fn eval_rules_parity() {
    let Some(expressir_dir) = expressir_dir() else {
        eprintln!("EXPRESSIR_DIR unset — skipping the rules parity run");
        return;
    };
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");

    let exp = read(&expressir_dir.join("spec/fixtures/eval/population.exp"));
    let p21 = read(&expressir_dir.join("spec/fixtures/eval/population.p21"));
    let gate_exp = read(&fixtures.join("eval_rules_gate.exp"));
    let gate_p21 = read(&fixtures.join("eval_rules_gate.p21"));

    let population_wire = schema_wire_of(&exp, "population.exp");
    let gate_wire = schema_wire_of(&gate_exp, "eval_rules_gate.exp");

    // (case id, schema wire, p21 source, mutations)
    let cases: Vec<(String, Value, String, Value)> = vec![
        (
            "clean".into(),
            population_wire.clone(),
            p21.clone(),
            json!([]),
        ),
        (
            "blank_label".into(),
            population_wire.clone(),
            p21.clone(),
            json!([{ "target": 1, "index": 0, "value": "" }]),
        ),
        (
            "same_edge_nodes".into(),
            population_wire.clone(),
            p21.clone(),
            json!([{ "target": 10, "index": 1, "value": "#1" }]),
        ),
        (
            "empty_graph".into(),
            population_wire.clone(),
            p21.clone(),
            json!([{ "target": 20, "index": 0, "value": { "list": [] } }]),
        ),
        (
            "unset_label".into(),
            population_wire.clone(),
            p21.clone(),
            json!([{ "target": 1, "index": 0, "value": "unset" }]),
        ),
        (
            "gate_clean".into(),
            gate_wire.clone(),
            gate_p21.clone(),
            json!([]),
        ),
        (
            "gate_string_code".into(),
            gate_wire.clone(),
            gate_p21.clone(),
            json!([{ "target": 1, "index": 0, "value": "" }]),
        ),
    ];

    let (ruby, harness_errors) = ruby_lanes(&expressir_dir, &cases).expect("ruby rules lane");
    assert!(
        harness_errors.is_empty(),
        "ruby harness raised: {harness_errors:?}"
    );

    let mut mismatches: Vec<String> = Vec::new();
    for (id, schema, _, mutations) in &cases {
        let expected = ruby.get(id).expect("ruby lane result");
        let mut population_request = expected["population"].clone();
        population_request["mutations"] = mutations.clone();
        let rust = match expressir::eval_rules::check_json(
            &schema.to_string(),
            &population_request.to_string(),
        ) {
            Ok(result) => serde_json::from_str::<Value>(&result).expect("result json"),
            Err(message) => json!({ "error": message }),
        };
        if rust["results"] != expected["results"] {
            mismatches.push(format!(
                "{id}:\n  rust  ={}\n  ruby  ={}",
                serde_json::to_string(&rust["results"]).unwrap_or_default(),
                serde_json::to_string(&expected["results"]).unwrap_or_default(),
            ));
        }
    }

    if !mismatches.is_empty() {
        panic!("rules parity mismatches:\n{}", mismatches.join("\n"));
    }
}
