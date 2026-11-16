//! The #640 uos-writer differential: the rs clause 9 uos emit must be
//! BYTE-identical to the ruby `Expressir::P28::Uos.emit` over the
//! fixture pairs — the delegation hands the ruby lane's own
//! `P21::Json.to_value` population wire and the schema `to_hash` to
//! the Rust writer, so the comparison isolates the writer (parse
//! parity is owned by the wire gates). Gated on EXPRESSIR_DIR like
//! the other no-drift tests.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::{json, Value};

const RUBY_HARNESS: &str = r##"
require "json"
root = ARGV[0]
$LOAD_PATH.unshift(File.join(root, "lib"))
require "expressir"

out = {}
request = JSON.parse($stdin.read)
request["pairs"].each do |key, pair|
  begin
    schema = Expressir::Express::Parser.from_exp(pair["schema"]).schemas.first
    population = Expressir::P21::Parser.parse(pair["population"])
    options = pair["options"] || {}
    xml = Expressir::P28::Uos.emit(
      population, schema,
      id: options["id"] || "uos01",
      schema_ref: options["schema_ref"],
      target_namespace: options["target_namespace"] ||
        Expressir::P28.default_target_namespace(schema.id),
      prefix: options["prefix"] || Expressir::P28::Uos.default_prefix(schema),
    )
    out[key] = {
      "xml" => xml,
      "population_wire" => Expressir::P21::Json.to_value(population),
      "schema_wire" => schema.to_hash,
      "options" => {
        "id" => options["id"] || "uos01",
        "schema_ref" => options["schema_ref"],
        "target_namespace" => options["target_namespace"] ||
          Expressir::P28.default_target_namespace(schema.id),
        "prefix" => options["prefix"] || Expressir::P28::Uos.default_prefix(schema),
      },
    }
  rescue StandardError, NotImplementedError => e
    out[key] = { "error" => "#{e.class}: #{e.message}" }
  end
end
puts JSON.generate(out)
"##;

const CAR_OWNERSHIP_SCHEMA: &str = "SCHEMA car_ownership;\n  ENTITY person;\n    name : STRING;\n    owns : car;\n  END_ENTITY;\n  ENTITY car;\n    make : STRING;\n    car_model : STRING;\n  END_ENTITY;\nEND_SCHEMA;\n";

const CAR_OWNERSHIP_POPULATION: &str = "ISO-10303-21;\nHEADER;\nFILE_NAME('p21.pop','2026-10-22T00:00:00',(''),(''),'','','');\nFILE_SCHEMA(('CAR_OWNERSHIP'));\nENDSEC;\nDATA;\n#10=PERSON('John Q. Public',#20);\n#20=CAR('BMW','Z3');\nENDSEC;\nEND-ISO-10303-21;\n";

/// Aggregates of simple and entity base, a typed parameter, and an
/// unset slot.
const AGGREGATE_SCHEMA: &str = "SCHEMA aggregates;\n  ENTITY curve_set;\n    points : LIST [1:?] OF point;\n    labels : LIST [1:?] OF STRING;\n    weights : ARRAY [1:3] OF REAL;\n  END_ENTITY;\n  ENTITY point;\n    x : REAL;\n    y : REAL;\n  END_ENTITY;\nEND_SCHEMA;\n";

const AGGREGATE_POPULATION: &str = "ISO-10303-21;\nHEADER;\nFILE_NAME('a.pop','2026-11-16T00:00:00',(''),(''),'','','');\nFILE_SCHEMA(('AGGREGATES'));\nENDSEC;\nDATA;\n#1=POINT(1.5,2.5);\n#2=POINT(0.0,1.0);\n#3=CURVE_SET((#1,#2),('a','b'),(1.0,2.0,3.0));\n#4=CURVE_SET($,$,$);\nENDSEC;\nEND-ISO-10303-21;\n";

/// A complex entity instance and a typed parameter. The aggregate keeps to
/// simple members: the ruby aggregate_element calls `.keyword` on complex
/// members and crashes (a latent ruby-lane bug), so the parity surface pins
/// the supported form.
const COMPLEX_SCHEMA: &str = "SCHEMA complex_parts;\n  ENTITY base_part;\n    id_label : STRING;\n  END_ENTITY;\n  ENTITY sized_part SUBTYPE OF (base_part);\n    size : REAL;\n  END_ENTITY;\n  ENTITY assembly;\n    parts : LIST [1:?] OF base_part;\n    tag : identification_tag;\n  END_ENTITY;\n  TYPE identification_tag = SELECT (tag_code, tag_number);\n  END_TYPE;\n  TYPE tag_code = STRING;\n  END_TYPE;\n  TYPE tag_number = INTEGER;\n  END_TYPE;\nEND_SCHEMA;\n";

const COMPLEX_POPULATION: &str = "ISO-10303-21;\nHEADER;\nFILE_NAME('c.pop','2026-11-16T00:00:00',(''),(''),'','','');\nFILE_SCHEMA(('COMPLEX_PARTS'));\nENDSEC;\nDATA;\n#1=BASE_PART('p1');\n#2=SIZED_PART('p2',3.5);\n#3=(BASE_PART('px')SIZED_PART(1.25));\n#4=ASSEMBLY((#1,#2),TAG_CODE('K1'));\nENDSEC;\nEND-ISO-10303-21;\n";

fn pairs() -> serde_json::Map<String, Value> {
    let mut pairs = serde_json::Map::new();
    pairs.insert(
        "worked_example".to_string(),
        json!({
            "schema": CAR_OWNERSHIP_SCHEMA,
            "population": CAR_OWNERSHIP_POPULATION,
        }),
    );
    pairs.insert(
        "aggregates".to_string(),
        json!({
            "schema": AGGREGATE_SCHEMA,
            "population": AGGREGATE_POPULATION,
        }),
    );
    pairs.insert(
        "complex_and_typed".to_string(),
        json!({
            "schema": COMPLEX_SCHEMA,
            "population": COMPLEX_POPULATION,
        }),
    );
    pairs.insert(
        "explicit_options".to_string(),
        json!({
            "schema": CAR_OWNERSHIP_SCHEMA,
            "population": CAR_OWNERSHIP_POPULATION,
            "options": {
                "id": "expdata42",
                "schema_ref": "s1",
                "target_namespace": "urn:demo:car",
                "prefix": "veh",
            }
        }),
    );
    pairs
}

fn expressir_dir() -> Option<PathBuf> {
    std::env::var_os("EXPRESSIR_DIR").map(PathBuf::from)
}

fn ruby_emissions(expressir_dir: &PathBuf) -> Result<Value, String> {
    let request = json!({ "pairs": pairs() });
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
    serde_json::from_slice(&output.stdout).map_err(|e| format!("json: {e}"))
}

#[test]
fn uos_emit_matches_the_ruby_writer_byte_for_byte() {
    let Some(expressir_dir) = expressir_dir() else {
        eprintln!("EXPRESSIR_DIR not set; skipping");
        return;
    };
    let ruby = ruby_emissions(&expressir_dir).expect("ruby emissions");

    let mut failures = Vec::new();
    for (key, _pair) in pairs() {
        let Some(entry) = ruby.get(&key) else {
            failures.push(format!("{key}: no ruby response"));
            continue;
        };
        if let Some(error) = entry.get("error").and_then(Value::as_str) {
            failures.push(format!("{key}: ruby lane errored: {error}"));
            continue;
        }
        let options = serde_json::to_string(&entry["options"]).expect("options");
        let rs_xml = expressir_rs::p28::uos_xml_json(
            &serde_json::to_string(&entry["population_wire"]).expect("population wire"),
            &serde_json::to_string(&entry["schema_wire"]).expect("schema wire"),
            &options,
        )
        .unwrap_or_else(|e| format!("rs error: {e}"));
        let ruby_xml = entry["xml"].as_str().unwrap_or("");
        if rs_xml != ruby_xml {
            failures.push(format!(
                "{key}: byte drift\n--- ruby ---\n{ruby_xml}\n--- rs ---\n{rs_xml}"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "uos writer drift:\n{}",
        failures.join("\n")
    );
}
