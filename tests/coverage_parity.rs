//! The #645 coverage differential: the rs coverage engine must
//! classify entities EXACTLY like the ruby Expressir::Coverage engine
//! over the corpus — per schema, the same walk order, the same
//! documented/undocumented split, the same {type, name} rows, the same
//! skip semantics (simple names, TYPE:SUBTYPE, FUNCTION:INNER), the
//! same coverage float. The delegation zips the rs rows with the
//! ruby-side model objects, so this order contract is load-bearing.
//!
//! The ruby harness parses each file on the production lane
//! (Parser.from_file — the remark attach pass runs) and emits both the
//! repository wire and the engine's rows per skip variant; the rs side
//! parses the same source, attaches remarks natively, and runs the
//! engine over its wire. Gated on EXPRESSIR_DIR like the other
//! no-drift tests.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

const RUBY_HARNESS: &str = r##"
require "json"
root = ARGV[0]
$LOAD_PATH.unshift(File.join(root, "lib"))
require "expressir"

fmt = lambda do |e|
  cn = e.class.name.split("::").last
  {"type" => Expressir::Coverage::CLASS_TO_EXPRESS_TYPE_MAP[cn] || cn.upcase,
   "name" => e.id.to_s}
end

out = {}
request = JSON.parse($stdin.read)
request["files"].each do |path, _source|
  begin
    repo = Expressir::Express::Parser.from_file(path)
    wire = repo.to_hash
    variants = {}
    request["variants"].each do |name, skips|
      schemas = repo.schemas.map do |schema|
        entities = Expressir::Coverage.find_entities(schema, skips)
        documented = entities.select { |e| Expressir::Coverage.entity_documented?(e) }
        undocumented = entities - documented
        {
          "id" => schema.id,
          "total" => entities.size,
          "documented" => documented.map(&fmt),
          "undocumented" => undocumented.map(&fmt),
          "coverage" => entities.empty? ? 100.0 : (documented.size.to_f / entities.size) * 100,
        }
      end
      variants[name] = schemas
    end
    out[path] = { "wire" => wire, "variants" => variants }
  rescue StandardError, NotImplementedError => e
    out[path] = { "error" => "#{e.class}: #{e.message}" }
  end
end
puts JSON.generate(out)
"##;

const VARIANTS: &[(&str, &[&str])] = &[
    ("none", &[]),
    ("type", &["TYPE"]),
    ("select", &["TYPE:SELECT"]),
    ("inner", &["FUNCTION:INNER"]),
    ("attrs", &["ENTITY", "PARAMETER"]),
];

/// The corpus is capped: the gate proves engine semantics, not
/// whole-corpus parse throughput, and the ruby harness holds every
/// wire in one process.
const CORPUS_CAP: usize = 120;

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
    files.truncate(CORPUS_CAP);
    files
}

fn ruby_reports(expressir_dir: &Path, sources: &BTreeMap<String, String>) -> Result<Value, String> {
    let variants: serde_json::Map<String, Value> = VARIANTS
        .iter()
        .map(|(name, skips)| {
            (
                name.to_string(),
                Value::Array(skips.iter().map(|s| json!(s)).collect()),
            )
        })
        .collect();
    let request = json!({ "files": sources, "variants": variants });
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

fn attached_wire(source: &str) -> Result<Value, String> {
    let tree = expressir::ParsedTree::parse(source).map_err(|e| format!("parse: {e}"))?;
    let mut wire = tree
        .to_model_json("parity.exp")
        .map_err(|e| format!("wire: {e:?}"))?;
    expressir::attach::attach_tagged_remarks(&mut wire, source, false);
    expressir::attach::attach_untagged_remarks(&mut wire, source, false);
    Ok(wire)
}

fn rows_of_kind(schema: &Value, kind: &str) -> Vec<(String, String)> {
    schema[kind]
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|r| {
                    (
                        r["type"].as_str().unwrap_or("").to_string(),
                        r["name"].as_str().unwrap_or("").to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn coverage_engine_matches_the_ruby_engine_over_the_corpus() {
    let Some(expressir_dir) = expressir_dir() else {
        eprintln!("EXPRESSIR_DIR not set; skipping");
        return;
    };
    let files = corpus_files(&expressir_dir);
    if files.is_empty() {
        eprintln!("no corpus files under EXPRESSIR_DIR; skipping");
        return;
    }

    let mut sources: BTreeMap<String, String> = Default::default();
    let mut parsed: Vec<(String, Value)> = Vec::new();
    for path in &files {
        let key = path.to_string_lossy().into_owned();
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(wire) = attached_wire(&source) else {
            continue;
        };
        sources.insert(key.clone(), source);
        parsed.push((key, wire));
    }
    if parsed.is_empty() {
        eprintln!("no parseable corpus files; skipping");
        return;
    }

    let ruby = ruby_reports(&expressir_dir, &sources).expect("ruby coverage");

    let mut failures: Vec<String> = Vec::new();
    let mut compared = 0usize;
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
        compared += 1;

        for (variant, skips) in VARIANTS {
            let skips: Vec<String> = skips.iter().map(|s| s.to_string()).collect();
            let rs = expressir::coverage::coverage(wire, &skips);
            let Some(ruby_schemas) = entry["variants"][variant].as_array() else {
                failures.push(format!("{key}/{variant}: ruby schemas missing"));
                continue;
            };
            if rs.schemas.len() != ruby_schemas.len() {
                failures.push(format!(
                    "{key}/{variant}: schema count ruby={} rs={}",
                    ruby_schemas.len(),
                    rs.schemas.len()
                ));
                continue;
            }
            for (rs_schema, ruby_schema) in rs.schemas.iter().zip(ruby_schemas) {
                let label = format!("{key}/{variant}/{}", rs_schema.id);
                if rs_schema.total != ruby_schema["total"].as_u64().unwrap_or(0) as usize {
                    failures.push(format!(
                        "{label}: total ruby={} rs={}",
                        ruby_schema["total"], rs_schema.total
                    ));
                }
                let rs_doc: Vec<(String, String)> = rs_schema
                    .rows
                    .iter()
                    .filter(|r| !r.skipped && r.documented)
                    .map(|r| (r.express_type.clone(), r.name.clone()))
                    .collect();
                if rs_doc != rows_of_kind(ruby_schema, "documented") {
                    failures.push(format!("{label}: documented rows drift"));
                }
                let rs_undoc: Vec<(String, String)> = rs_schema
                    .rows
                    .iter()
                    .filter(|r| !r.skipped && !r.documented)
                    .map(|r| (r.express_type.clone(), r.name.clone()))
                    .collect();
                if rs_undoc != rows_of_kind(ruby_schema, "undocumented") {
                    failures.push(format!("{label}: undocumented rows drift"));
                }
                // The percentage is derived data and the delegation
                // recomputes it ruby-side from the counts; the
                // comparison carries a 1e-9 tolerance because the
                // json gem's float formatting is not always
                // shortest-roundtrip (last-ULP artifacts).
                let rs_cov = rs_schema.coverage;
                let ruby_cov = ruby_schema["coverage"].as_f64().unwrap_or(f64::NAN);
                if (rs_cov - ruby_cov).abs() > 1e-9 {
                    failures.push(format!("{label}: coverage ruby={ruby_cov} rs={rs_cov}"));
                }
            }
        }
    }

    assert!(
        compared > 0,
        "no files compared — the corpus produced no paired reports"
    );
    assert!(
        failures.is_empty(),
        "coverage drift over {compared} files:\n{}",
        failures
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
