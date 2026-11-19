//! The #646 PR-2 inventory differential: the rs one-pass inventory
//! (parse + resolve + rows) must match the ruby Document's own walks —
//! declarations (id, class, selection byte) and references (id,
//! capture offset) — over the corpus. The base_path fills are owned by
//! the resolve parity gate; this gate adds the internal-consistency
//! check (every resolved reference's base_path names a declaration
//! path in the same inventory). Gated on EXPRESSIR_DIR.

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
request["documents"].each do |key, source|
  begin
    document = Expressir::Lsp::Document.new(uri: "file:///#{key}.exp", text: source)
    declarations = document.declaration_nodes.map do |node|
      {
        "id" => node.id.to_s,
        "class" => node.class.name,
        "sel_start" => document.name_offset(node),
      }
    end
    references = document.reference_nodes.map do |node|
      {
        "id" => node.id.to_s,
        "offset" => node.source_offset,
      }
    end
    out[key] = { "declarations" => declarations, "references" => references }
  rescue StandardError => e
    out[key] = { "error" => "#{e.class}: #{e.message}" }
  end
end
puts JSON.generate(out)
"##;

const CORPUS_CAP: usize = 40;

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

fn ruby_inventories(
    expressir_dir: &Path,
    documents: &serde_json::Map<String, Value>,
) -> Result<Value, String> {
    let request = json!({ "documents": documents });
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

fn doc_key(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("doc")
        .to_string()
}

/// IGNORED for one residual family: 39 of the 40 corpus documents
/// compare byte-exact under this gate; the synthetic concatenated
/// oracle fixture still shows rs capture offsets sitting ~20 bytes
/// later than the ruby builder's on some attribute/reference nodes —
/// the capture-START convention (rs begins at the subtree's first
/// token, ruby at the name). That is the remaining expressir-rs#91
/// item; the gate goes required when the builder stamps those nodes
/// from the name capture.
#[test]
fn lsp_inventory_matches_the_ruby_walks_over_the_corpus() {
    let Some(expressir_dir) = expressir_dir() else {
        eprintln!("EXPRESSIR_DIR not set; skipping");
        return;
    };
    let files = corpus_files(&expressir_dir);
    if files.is_empty() {
        eprintln!("no corpus files under EXPRESSIR_DIR; skipping");
        return;
    }

    let mut documents = serde_json::Map::new();
    let mut sources: Vec<(String, String)> = Vec::new();
    for path in &files {
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let key = doc_key(path);
        documents.insert(key.clone(), json!(source));
        sources.push((key, source));
    }
    if sources.is_empty() {
        eprintln!("no readable corpus documents; skipping");
        return;
    }

    let ruby = ruby_inventories(&expressir_dir, &documents).expect("ruby inventories");

    let mut failures = Vec::new();
    let mut compared = 0usize;
    for (key, source) in &sources {
        let Some(entry) = ruby.get(key) else {
            continue;
        };
        if let Some(error) = entry.get("error").and_then(Value::as_str) {
            failures.push(format!("{key}: ruby lane errored: {error}"));
            continue;
        }
        let inventory: Value = match serde_json::from_str(
            &expressir::lsp::inventory_json(source).expect("rs inventory"),
        ) {
            Ok(inventory) => inventory,
            Err(e) => {
                failures.push(format!("{key}: rs errored: {e}"));
                continue;
            }
        };
        compared += 1;

        // Declarations as (id, class, sel) triples — order-insensitive
        // (the collector order and the wire order need not agree).
        let ruby_decls: BTreeSet<(String, String, i64)> = entry["declarations"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|d| {
                (
                    d["id"].as_str().unwrap_or("").to_string(),
                    d["class"].as_str().unwrap_or("").to_string(),
                    d["sel_start"].as_i64().unwrap_or(-1),
                )
            })
            .collect();
        let rs_decls: BTreeSet<(String, String, i64)> = inventory["declarations"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|d| {
                (
                    d["id"].as_str().unwrap_or("").to_string(),
                    d["class"].as_str().unwrap_or("").to_string(),
                    d["sel_start"].as_i64().unwrap_or(-1),
                )
            })
            .collect();
        if rs_decls != ruby_decls {
            let only_ruby: Vec<_> = ruby_decls.difference(&rs_decls).take(5).collect();
            let only_rs: Vec<_> = rs_decls.difference(&ruby_decls).take(5).collect();
            failures.push(format!(
                "{key}: declarations drift (ruby={} rs={}) only-ruby={only_ruby:?} only-rs={only_rs:?}",
                ruby_decls.len(),
                rs_decls.len()
            ));
        }

        // References under the wire gates' position-intersection
        // rule: the rs builder stamps MORE references than the ruby
        // builder (the additive stamps) and a presence difference is
        // not drift — a both-present mismatch is. The id multisets
        // must agree, and each rs-offset must be either present on
        // the ruby side or an addition over a ruby None.
        let ref_ids = |rows: &Value| -> BTreeSet<String> {
            rows.as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|r| r["id"].as_str().unwrap_or("").to_string())
                .collect()
        };
        if ref_ids(&entry["references"]) != ref_ids(&inventory["references"]) {
            failures.push(format!("{key}: reference id sets drift"));
        }
        let mut ruby_by_id: std::collections::BTreeMap<String, Vec<Option<i64>>> =
            Default::default();
        for r in entry["references"].as_array().cloned().unwrap_or_default() {
            ruby_by_id
                .entry(r["id"].as_str().unwrap_or("").to_string())
                .or_default()
                .push(r["offset"].as_i64());
        }
        for r in inventory["references"]
            .as_array()
            .cloned()
            .unwrap_or_default()
        {
            let id = r["id"].as_str().unwrap_or("").to_string();
            let offset = r["offset"].as_i64();
            let Some(ruby_offsets) = ruby_by_id.get_mut(&id) else {
                continue;
            };
            if let Some(position) = ruby_offsets.iter().position(|o| *o == offset) {
                ruby_offsets.remove(position);
            } else if let Some(none_position) = ruby_offsets.iter().position(|o| o.is_none()) {
                // The stamp is additive: ruby left this reference
                // unpositioned and rs carries the byte — tolerated.
                ruby_offsets.remove(none_position);
            } else {
                failures.push(format!(
                    "{key}: reference {id} offset mismatch at {offset:?}"
                ));
            }
        }

        // Internal consistency: a resolved base_path must name a
        // declaration path of this inventory, or a non-symbol node the
        // fill legitimately targets (parameters of interfaced
        // functions, nested-scope variables). A miss whose tail is a
        // SYMBOL id means the walk dropped a node.
        let decl_values: Vec<Value> = inventory["declarations"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let paths: BTreeSet<&str> = decl_values
            .iter()
            .filter_map(|d| d["path"].as_str())
            .collect();
        let symbol_tails: BTreeSet<&str> = decl_values
            .iter()
            .filter_map(|d| d["path"].as_str().and_then(|p| p.rsplit('.').next()))
            .collect();
        for r in inventory["references"]
            .as_array()
            .cloned()
            .unwrap_or_default()
        {
            if let Some(base) = r["base_path"].as_str() {
                if !paths.contains(base) {
                    let tail = base.rsplit('.').next().unwrap_or("");
                    // A fill whose PARENT is a declared node targets a
                    // non-symbol member of it (a derived/inverse
                    // attribute, a parameter) — the ruby resolver fills
                    // those legitimately and the symbol declaration
                    // rows deliberately omit them (SYMBOL_KINDS).
                    let parent_declared = base
                        .rsplit_once('.')
                        .map(|(parent, _)| paths.contains(parent))
                        .unwrap_or(false);
                    if symbol_tails.contains(tail) && !parent_declared {
                        failures.push(format!("{key}: base_path {base} names no declaration path"));
                        break;
                    }
                }
            }
        }
    }

    assert!(
        compared > 0,
        "no documents compared — the corpus produced no paired inventories"
    );
    assert!(
        failures.is_empty(),
        "lsp inventory drift over {compared} documents:\n{}",
        failures
            .iter()
            .take(30)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
