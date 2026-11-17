//! The #646 PR-3 rename differential: the rs rename-edits engine must
//! produce the SAME edit positions as the ruby `Workspace#rename`
//! (declaration selection + resolved references, byte ranges), over
//! single-document workspaces. The ruby harness renames through the
//! Workspace's public face (position math included) and the comparison
//! converts its LSP ranges back to bytes with the ruby-side math —
//! the exact conversion the delegation runs. Gated on EXPRESSIR_DIR.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

const RUBY_HARNESS: &str = r##"
require "json"
ENV["EXPRESSIR_DISABLE_NATIVE"] = "1"
root = ARGV[0]
$LOAD_PATH.unshift(File.join(root, "lib"))
require "expressir"

out = {}
request = JSON.parse($stdin.read)
request["cases"].each do |key, case_data|
  begin
    workspace = Expressir::Lsp::Workspace.new
    workspace.open(uri: "file:///#{key}.exp", text: case_data["source"])
    document = workspace.document("file:///#{key}.exp")
    target = document.declaration_nodes.find do |node|
      node.id.to_s.casecmp?(case_data["target"])
    end
    raise "no declaration #{case_data["target"]}" unless target

    position = document.position(document.name_offset(target))
    result = workspace.rename("file:///#{key}.exp", position, case_data["new_name"])
    edits = result ? result["changes"]["file:///#{key}.exp"] : nil
    out[key] = {
      "edits" => edits || [],
      "renamed" => !result.nil?,
    }
  rescue StandardError => e
    out[key] = { "error" => "#{e.class}: #{e.message}" }
  end
end
puts JSON.generate(out)
"##;

const SUBTYPE_SOURCE: &str = "SCHEMA shapes;\n  ENTITY base_shape;\n  END_ENTITY;\n  ENTITY area_shape SUBTYPE OF (base_shape);\n    area : base_shape_count;\n  END_ENTITY;\n  TYPE base_shape_count = INTEGER;\n  END_TYPE;\nEND_SCHEMA;\n";

const INTERFACE_SOURCE: &str = "SCHEMA providers;\n  ENTITY shared_part;\n  END_ENTITY;\nEND_SCHEMA;\nSCHEMA consumers;\n  USE FROM providers (shared_part AS item);\n  ENTITY holder;\n    part : item;\n  END_ENTITY;\nEND_SCHEMA;\n";

fn cases() -> serde_json::Map<String, Value> {
    let mut cases = serde_json::Map::new();
    cases.insert(
        "subtype_chain".to_string(),
        json!({
            "source": SUBTYPE_SOURCE,
            "target": "base_shape",
            "new_name": "primitive_shape",
            "schema": "shapes",
        }),
    );
    cases.insert(
        "interface_rename".to_string(),
        json!({
            "source": INTERFACE_SOURCE,
            "target": "shared_part",
            "new_name": "common_part",
            "schema": "providers",
        }),
    );
    cases
}

fn expressir_dir() -> Option<PathBuf> {
    std::env::var_os("EXPRESSIR_DIR").map(PathBuf::from)
}

fn ruby_renames(expressir_dir: &Path) -> Result<Value, String> {
    let request = json!({ "cases": cases() });
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

/// The ruby-side byte math (line starts + UTF-16 columns), converting
/// an LSP range back to a byte span — the inverse of the delegation's
/// conversion.
fn line_starts(bytes: &[u8]) -> Vec<usize> {
    let mut starts = vec![0usize];
    let mut index = 0usize;
    while let Some(found) = bytes[index..].iter().position(|b| *b == b'\n') {
        index += found + 1;
        starts.push(index);
    }
    starts
}

fn utf16_code_units(ch: char) -> usize {
    if (ch as u32) > 0xFFFF {
        2
    } else {
        1
    }
}

fn byte_of(bytes: &[u8], starts: &[usize], line: usize, character: usize) -> usize {
    let Some(&start) = starts.get(line) else {
        return bytes.len();
    };
    let line_end = starts
        .get(line + 1)
        .map(|next| next.saturating_sub(1))
        .unwrap_or(bytes.len());
    let text = String::from_utf8_lossy(&bytes[start..line_end]).to_string();
    let mut units = 0usize;
    let mut byte = start;
    for ch in text.chars() {
        if units >= character {
            break;
        }
        units += utf16_code_units(ch);
        byte += ch.len_utf8();
    }
    byte
}

/// IGNORED pending capture parity (expressir-rs#91): the id-filtered
/// engine matches the ruby Workspace on every case whose references
/// carry capture offsets; the SUBTYPE OF reference case still drifts
/// because the rs builder leaves subtype references unpositioned (the
/// offset-stamping family from the inventory gate). The gate goes
/// required when that family lands.
#[test]

fn lsp_rename_edits_match_the_ruby_workspace_byte_for_byte() {
    let Some(expressir_dir) = expressir_dir() else {
        eprintln!("EXPRESSIR_DIR not set; skipping");
        return;
    };
    let ruby = ruby_renames(&expressir_dir).expect("ruby renames");

    let mut failures = Vec::new();
    for (key, case_data) in cases() {
        let Some(entry) = ruby.get(&key) else {
            failures.push(format!("{key}: no ruby response"));
            continue;
        };
        if let Some(error) = entry.get("error").and_then(Value::as_str) {
            failures.push(format!("{key}: ruby lane errored: {error}"));
            continue;
        }
        let source = case_data["source"].as_str().expect("source");
        let schema = case_data["schema"].as_str().expect("schema");
        let target = case_data["target"].as_str().expect("target");
        let path = format!("document.exp.{schema}.{target}");
        let edits: Value = match serde_json::from_str(
            &expressir_rs::lsp::rename_edits_json(source, &path).expect("rs edits"),
        ) {
            Ok(edits) => edits,
            Err(e) => {
                failures.push(format!("{key}: rs errored: {e}"));
                continue;
            }
        };

        let bytes = source.as_bytes();
        let starts = line_starts(bytes);
        let rs_spans: BTreeSet<(usize, usize)> = edits
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|edit| {
                let start = edit["sel_start"].as_u64().unwrap_or(0) as usize;
                let len = edit["sel_len"].as_u64().unwrap_or(0) as usize;
                (start, start + len)
            })
            .collect();
        let ruby_spans: BTreeSet<(usize, usize)> = entry["edits"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|edit| {
                let start = edit["range"]["start"].clone();
                let end = edit["range"]["end"].clone();
                let a = byte_of(
                    bytes,
                    &starts,
                    start["line"].as_u64().unwrap_or(0) as usize,
                    start["character"].as_u64().unwrap_or(0) as usize,
                );
                let b = byte_of(
                    bytes,
                    &starts,
                    end["line"].as_u64().unwrap_or(0) as usize,
                    end["character"].as_u64().unwrap_or(0) as usize,
                );
                (a, b)
            })
            .collect();

        if rs_spans != ruby_spans {
            failures.push(format!(
                "{key}: edit drift ruby={ruby_spans:?} rs={rs_spans:?}"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "lsp rename drift:\n{}",
        failures.join("\n")
    );
}
