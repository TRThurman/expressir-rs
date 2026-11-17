//! Rust-vs-Ruby population-scale parity (expressir#639, slice 4).
//!
//! Both lanes run `Eval::Population.check` — WHERE rules scoped over
//! entities and their subtypes via typing, UNIQUE rules (9.2.2.1)
//! across the population, and global rules (9.4) with the FOR
//! entities bound to their instance aggregates. The Ruby lane parses
//! the fixture sources, emits the gate population encoding, and
//! checks; the Rust lane replays the emitted population through
//! `eval_rules::population_check_json` over the rs schema wire.
//! Skipped without `EXPRESSIR_DIR`.

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
    checked = Expressir::Eval::Population.check(schema, population)
    results[c["id"]] = {
      "population" => encoded,
      "findings" => checked.map do |f|
        {
          "kind" => f.kind.to_s,
          "instance" => f.instance,
          "rule_id" => f.rule_id,
          "message" => f.message,
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

fn ruby_lanes(expressir_dir: &Path, cases: &[(String, Value, String)]) -> LaneMaps {
    let request = json!({
        "cases": cases
            .iter()
            .map(|(id, schema, p21)| {
                json!({ "id": id, "schema_file": schema, "p21": p21 })
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

fn p21_for(body: &str) -> String {
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('','','','','','','');\nFILE_SCHEMA(('pop_demo'));\nENDSEC;\nDATA;\n{body}\nENDSEC;\nEND-ISO-10303-21;\n"
    )
}

#[test]
fn eval_population_parity() {
    let Some(expressir_dir) = expressir_dir() else {
        eprintln!("EXPRESSIR_DIR unset — skipping the population parity run");
        return;
    };

    let schema_source = r#"SCHEMA pop_demo;
  ENTITY base ABSTRACT SUPERTYPE;
    code : INTEGER;
  END_ENTITY;
  ENTITY material SUBTYPE OF (base);
    UNIQUE u1 : code;
    WHERE nonneg : code >= 0;
  END_ENTITY;
  ENTITY special SUBTYPE OF (material);
  END_ENTITY;
  ENTITY document SUBTYPE OF (base);
    text : STRING;
  END_ENTITY;
  ENTITY blob;
    n : INTEGER;
  END_ENTITY;
  RULE material_codes_nonneg FOR (material);
    WHERE
      ok : SIZEOF(QUERY(m <* material | m.code < 0)) = 0;
  END_RULE;
END_SCHEMA;"#;

    let tree = expressir_rs::ParsedTree::parse(schema_source).expect("rs lane parses the schema");
    let wire = tree.to_model_json("pop_demo.exp").expect("rs wire");
    let schema_wire = &wire["schemas"][0];

    let cases: Vec<(String, String)> = vec![
        (
            "clean".into(),
            p21_for("#1=MATERIAL(5);\n#2=MATERIAL(7);\n#3=DOCUMENT('spec');\n#4=BLOB(-2);"),
        ),
        ("subtype_where".into(), p21_for("#5=SPECIAL(-1);")),
        (
            "unique_dup".into(),
            p21_for("#1=MATERIAL(9);\n#2=SPECIAL(9);"),
        ),
        (
            "unique_distinct".into(),
            p21_for("#1=MATERIAL(9);\n#2=SPECIAL(10);"),
        ),
        (
            "global_violation".into(),
            p21_for("#1=MATERIAL(5);\n#2=SPECIAL(-1);"),
        ),
        ("global_clean".into(), p21_for("#1=MATERIAL(5);")),
    ];

    let cases_with_schema: Vec<(String, Value, String)> = cases
        .iter()
        .map(|(id, p21)| (id.clone(), schema_wire.clone(), p21.clone()))
        .collect();

    let (ruby, harness_errors) =
        ruby_lanes(&expressir_dir, &cases_with_schema).expect("ruby population lane");
    assert!(
        harness_errors.is_empty(),
        "ruby harness raised: {harness_errors:?}"
    );

    let mut mismatches: Vec<String> = Vec::new();
    for (id, _) in &cases {
        let expected = ruby.get(id).expect("ruby lane result");
        let population_request = expected["population"].clone();
        let rust = match expressir_rs::eval_rules::population_check_json(
            &schema_wire.to_string(),
            &population_request.to_string(),
        ) {
            Ok(result) => serde_json::from_str::<Value>(&result).expect("result json"),
            Err(message) => json!({ "error": message }),
        };
        if rust["findings"] != expected["findings"] {
            mismatches.push(format!(
                "{id}:\n  rust  ={}\n  ruby  ={}",
                serde_json::to_string(&rust["findings"]).unwrap_or_default(),
                serde_json::to_string(&expected["findings"]).unwrap_or_default(),
            ));
        }
    }

    if !mismatches.is_empty() {
        panic!("population parity mismatches:\n{}", mismatches.join("\n"));
    }
}
