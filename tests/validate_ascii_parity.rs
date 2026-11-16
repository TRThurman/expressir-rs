//! The #643 differential: the rs ascii scan must produce the SAME
//! violations (run text, line, character column, char details with the
//! replacement suggestions) as the ruby `validate ascii` engine over
//! the non-ASCII corpus. Gated on EXPRESSIR_DIR like the other
//! no-drift tests.

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
request["files"].each do |path, _source|
  collection = Expressir::Commands::NonAsciiViolationCollection.new
  collection.process_file(path)
  file_violations = collection.file_violations[path]
  out[path] = file_violations ? file_violations.violations : []
end
puts JSON.generate(out)
"##;

fn expressir_dir() -> Option<PathBuf> {
    std::env::var_os("EXPRESSIR_DIR").map(PathBuf::from)
}

fn collect_sources(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    fn walk(dir: &Path, out: &mut Vec<(String, String)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "exp") {
                if let Ok(source) = std::fs::read_to_string(&path) {
                    if source.is_char_boundary(1) && source.chars().any(|c| (c as u32) > 0x7F) {
                        out.push((path.to_string_lossy().into_owned(), source));
                    }
                }
            }
        }
    }
    walk(root, &mut out);
    out.sort();
    out
}

fn ruby_violations(
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
fn ascii_scan_matches_the_ruby_engine_over_the_corpus() {
    let Some(expressir_dir) = expressir_dir() else {
        eprintln!("EXPRESSIR_DIR not set; skipping");
        return;
    };
    let sources = collect_sources(&expressir_dir);
    if sources.is_empty() {
        eprintln!("no non-ASCII corpus files; skipping");
        return;
    }
    let ruby = ruby_violations(&expressir_dir, &sources.iter().cloned().collect())
        .expect("ruby violations");

    let mut failures = Vec::new();
    for (path, source) in &sources {
        let rust_rows = expressir_rs::validate_ascii::scan_source(source, false);
        let Some(ruby_rows) = ruby.get(path).and_then(Value::as_array) else {
            continue;
        };
        if rust_rows.len() != ruby_rows.len() {
            failures.push(format!(
                "{path}: violation count ruby={} rs={}",
                ruby_rows.len(),
                rust_rows.len()
            ));
            continue;
        }
        for (rr, vr) in ruby_rows.iter().zip(&rust_rows) {
            let ruby_line = rr.get("line_number").and_then(Value::as_u64);
            let rs_line = serde_json::to_value(vr.line_number)
                .ok()
                .and_then(|v| v.as_u64());
            if ruby_line != rs_line {
                failures.push(format!("{path}: line ruby={ruby_line:?} rs={rs_line:?}"));
            }
            let ruby_match = rr.get("match").and_then(Value::as_str);
            if ruby_match != Some(vr.match_text.as_str()) {
                failures.push(format!(
                    "{path}: match ruby={ruby_match:?} rs={:?}",
                    vr.match_text
                ));
            }
            let ruby_details = rr.get("char_details").and_then(Value::as_array);
            match ruby_details {
                Some(details) if details.len() == vr.char_details.len() => {
                    for (rd, cd) in details.iter().zip(&vr.char_details) {
                        let ours = serde_json::to_value(cd).expect("serialize detail");
                        for key in [
                            "char",
                            "hex",
                            "utf8",
                            "is_math",
                            "replacement",
                            "replacement_type",
                        ] {
                            let rv = rd.get(key);
                            let sv = ours.get(key);
                            if rv != sv {
                                failures.push(format!("{path}: {key} ruby={rv:?} rs={sv:?}"));
                            }
                        }
                    }
                }
                Some(details) => failures.push(format!(
                    "{path}: char_details count ruby={} rs={}",
                    details.len(),
                    vr.char_details.len()
                )),
                None => failures.push(format!("{path}: ruby char_details missing")),
            }
        }
    }
    assert!(
        failures.is_empty(),
        "ascii scan drift over {} files:\n{}",
        sources.len(),
        failures
            .iter()
            .take(30)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
