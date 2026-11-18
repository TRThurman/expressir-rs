//! The #641 xsd-writer differential: the rs XSD writer must be
//! BYTE-identical to the ruby `Expressir::Express::Xsd.format` over
//! the fixture schemas. The ruby harness parses on the production
//! lane (from_file — reference resolution fills base_path, which the
//! writer consumes), hands over the schema's to_hash, and forces the
//! pure-ruby lane so the gate never compares the engine to itself
//! once the expressir delegation lands. Gated on EXPRESSIR_DIR.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::{json, Value};

const RUBY_HARNESS: &str = r##"
require "json"
require "tempfile"
ENV["EXPRESSIR_DISABLE_NATIVE"] = "1"
root = ARGV[0]
$LOAD_PATH.unshift(File.join(root, "lib"))
require "expressir"

out = {}
request = JSON.parse($stdin.read)
request["schemas"].each do |key, source|
  begin
    Tempfile.create(["xsd_parity", ".exp"]) do |file|
      file.write(source)
      file.flush
      schema = Expressir::Express::Parser.from_file(file.path).schemas.first
      out[key] = {
        "xml" => Expressir::Express::Xsd.format(schema),
        "schema_wire" => schema.to_hash,
      }
    end
  rescue StandardError, NotImplementedError => e
    out[key] = { "error" => "#{e.class}: #{e.message}" }
  end
end
puts JSON.generate(out)
"##;

const BUILTINS: &str = "SCHEMA primitives;\n  TYPE colour = ENUMERATION OF (red, green);\n  END_TYPE;\n  TYPE mixed = SELECT (colour, point, label);\n  END_TYPE;\n  TYPE open_selection = SELECT;\n  END_TYPE;\n  TYPE label = STRING;\n  END_TYPE;\n  TYPE coordinates = LIST [1:3] OF REAL;\n  END_TYPE;\n  ENTITY point;\n    x : REAL;\n    y : REAL;\n    tag : OPTIONAL label;\n  END_ENTITY;\n  ENTITY blob;\n    body : BINARY;\n    flag : BOOLEAN;\n    state : LOGICAL;\n    parts : SET OF point;\n  END_ENTITY;\nEND_SCHEMA;\n";

const INHERITANCE: &str = "SCHEMA shapes;\n  ENTITY shape;\n    name : STRING;\n  END_ENTITY;\n  ENTITY area_shape SUBTYPE OF (shape);\n    area : REAL;\n  END_ENTITY;\n  ENTITY circle SUBTYPE OF (area_shape);\n    radius : REAL;\n  END_ENTITY;\nEND_SCHEMA;\n";

const MULTIPLE_INHERITANCE: &str = "SCHEMA hybrids;\n  ENTITY base_a;\n    a_attr : INTEGER;\n  END_ENTITY;\n  ENTITY base_b;\n    b_attr : INTEGER;\n    shared : REAL;\n  END_ENTITY;\n  ENTITY base_c SUBTYPE OF (base_b);\n    c_attr : REAL;\n  END_ENTITY;\n  ENTITY hybrid SUBTYPE OF (base_a, base_c);\n    extra : STRING;\n  END_ENTITY;\nEND_SCHEMA;\n";

fn schemas() -> serde_json::Map<String, Value> {
    let mut schemas = serde_json::Map::new();
    schemas.insert("builtins".to_string(), json!(BUILTINS));
    schemas.insert("inheritance".to_string(), json!(INHERITANCE));
    schemas.insert(
        "multiple_inheritance".to_string(),
        json!(MULTIPLE_INHERITANCE),
    );
    schemas
}

fn expressir_dir() -> Option<PathBuf> {
    std::env::var_os("EXPRESSIR_DIR").map(PathBuf::from)
}

fn ruby_emissions(expressir_dir: &PathBuf) -> Result<Value, String> {
    let request = json!({ "schemas": schemas() });
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
fn xsd_format_matches_the_ruby_writer_byte_for_byte() {
    let Some(expressir_dir) = expressir_dir() else {
        eprintln!("EXPRESSIR_DIR not set; skipping");
        return;
    };
    let ruby = ruby_emissions(&expressir_dir).expect("ruby emissions");

    let mut failures = Vec::new();
    for key in schemas().keys() {
        let Some(entry) = ruby.get(key) else {
            failures.push(format!("{key}: no ruby response"));
            continue;
        };
        if let Some(error) = entry.get("error").and_then(Value::as_str) {
            failures.push(format!("{key}: ruby lane errored: {error}"));
            continue;
        }
        let rs_xml = expressir::xsd::xsd_xml_json(
            &serde_json::to_string(&entry["schema_wire"]).expect("schema wire"),
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
        "xsd writer drift:\n{}",
        failures.join("\n")
    );
}
