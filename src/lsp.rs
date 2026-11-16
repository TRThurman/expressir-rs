//! The LSP document-symbols engine (#646 PR-1): a port of the ruby
//! `Document#symbols` walk (NodePositionIndex semantics) over a native
//! parse of the document text. The delegation hands back rows in BYTE
//! terms — 1-based span lines plus the selection byte window — and the
//! ruby Document converts them through its own position math (LSP
//! UTF-16 positions stay transport-side). The parity gate compares the
//! rows against the ruby walk; the differential caught nothing so far,
//! which is the point of keeping it.

use serde_json::Value;

/// The ruby Document::SYMBOL_KINDS class → LSP symbol kind.
fn symbol_kind(class: &str) -> Option<u8> {
    match class {
        "Expressir::Model::Declarations::Schema" => Some(3),
        "Expressir::Model::Declarations::Entity" => Some(5),
        "Expressir::Model::Declarations::Type" => Some(26),
        "Expressir::Model::Declarations::Function" => Some(12),
        "Expressir::Model::Declarations::Procedure" => Some(6),
        "Expressir::Model::Declarations::Rule" => Some(11),
        "Expressir::Model::Declarations::Constant" => Some(14),
        "Expressir::Model::Declarations::Attribute" => Some(8),
        "Expressir::Model::Declarations::Variable" => Some(13),
        _ => None,
    }
}

fn class_of(node: &Value) -> &str {
    node.get("_class").and_then(Value::as_str).unwrap_or("")
}

/// The ruby LineMap: 1-based line numbers over byte offsets.
struct LineMap {
    starts: Vec<usize>,
}

impl LineMap {
    fn new(bytes: &[u8]) -> Self {
        let mut starts = vec![0usize];
        let mut index = 0usize;
        while let Some(found) = bytes[index..].iter().position(|b| *b == b'\n') {
            index += found + 1;
            starts.push(index);
        }
        LineMap { starts }
    }

    fn line_number(&self, byte: usize) -> usize {
        match self.starts.binary_search(&byte) {
            Ok(i) => i + 1,
            Err(i) => i,
        }
    }
}

/// One positioned node in DFS order, mirroring the ruby entry shape.
struct Entry {
    class: String,
    id: Option<String>,
    is_attribute: bool,
    is_scope: bool,
    valid: bool,
    start: u64,
    end: u64,
    end_line: usize,
    parent: Option<usize>,
}

/// The declaration-level leaves the ruby grammar lane positions with
/// named spans — the children whose ends raise their scope (items,
/// references and data-type captures are not positioned there and
/// never raise).
fn is_raiser_class(class: &str) -> bool {
    is_scope_class(class)
        || matches!(
            class,
            "Expressir::Model::Declarations::Attribute"
                | "Expressir::Model::Declarations::WhereRule"
                | "Expressir::Model::Declarations::UniqueRule"
                | "Expressir::Model::Declarations::InformalPropositionRule"
                | "Expressir::Model::Declarations::Parameter"
                | "Expressir::Model::Declarations::Variable"
        )
}

/// The classes whose declarations own an END keyword — the scope
/// containers the ruby propagation exists to fix.
fn is_scope_class(class: &str) -> bool {
    matches!(
        class,
        "Expressir::Model::Declarations::Schema"
            | "Expressir::Model::Declarations::Entity"
            | "Expressir::Model::Declarations::Type"
            | "Expressir::Model::Declarations::Function"
            | "Expressir::Model::Declarations::Procedure"
            | "Expressir::Model::Declarations::Rule"
            | "Expressir::Model::Declarations::Constant"
            | "Expressir::Model::Declarations::Variable"
    )
}

/// The ruby position_valid?: byte-0 starts are only accepted for
/// declaration-keyword-leading slices.
fn position_valid(source: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "SCHEMA",
        "ENTITY",
        "TYPE",
        "FUNCTION",
        "PROCEDURE",
        "RULE",
        "CONSTANT",
        "VARIABLE",
        "USE",
        "REFERENCE",
        "END_SCHEMA",
        "END_ENTITY",
        "END_TYPE",
        "END_FUNCTION",
        "END_PROCEDURE",
        "END_RULE",
        "END_CONSTANT",
        "END_VARIABLE",
    ];
    KEYWORDS.iter().any(|keyword| source.starts_with(keyword))
}

/// The ruby name_offset: search a 1024-byte window from the node's
/// offset for `id` with word boundaries, case-insensitive; the match
/// position is the WINDOW's character index (the ruby regex runs on a
/// UTF-8 slice), mirrored exactly.
fn name_offset(window: &[u8], offset: usize, id: &str) -> Option<usize> {
    let id_bytes = id.as_bytes();
    if id_bytes.is_empty() || id_bytes.len() > window.len() {
        return None;
    }
    let boundary = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    for start in 0..=(window.len() - id_bytes.len()) {
        if !window[start..start + id_bytes.len()].eq_ignore_ascii_case(id_bytes) {
            continue;
        }
        let before_ok = start == 0 || !boundary(window[start - 1]);
        let end = start + id_bytes.len();
        let after_ok = end == window.len() || !boundary(window[end]);
        if before_ok && after_ok {
            let char_index = String::from_utf8_lossy(&window[..start]).chars().count();
            return Some(offset + char_index);
        }
    }
    None
}

fn walk(node: &Value, parent: Option<usize>, rows: &mut Vec<Entry>) {
    let Value::Object(map) = node else { return };
    let class = class_of(node).to_string();
    if !class.is_empty() {
        let start = map.get("source_offset").and_then(Value::as_u64);
        let end = map.get("source_end_offset").and_then(Value::as_u64);
        if let (Some(start), Some(end)) = (start, end) {
            let source = map.get("source").and_then(Value::as_str).unwrap_or("");
            let valid = start > 0 || (!source.is_empty() && position_valid(source));
            rows.push(Entry {
                is_attribute: class == "Expressir::Model::Declarations::Attribute",
                is_scope: is_scope_class(&class),
                id: map.get("id").and_then(Value::as_str).map(str::to_string),
                valid,
                start,
                end,
                end_line: 0,
                class,
                parent,
            });
            let parent_index = Some(rows.len() - 1);
            for value in map.values() {
                walk_value(value, parent_index, rows);
            }
            return;
        }
        let parent_index = if rows.last().is_some() {
            // Unpositioned nodes still parent their subtree to the
            // nearest positioned ancestor for the walk, but the ruby
            // index keeps them line-less; their subtree parents to
            // this node only when it was recorded.
            None
        } else {
            parent
        };
        for value in map.values() {
            walk_value(value, parent_index, rows);
        }
        return;
    }
    for value in map.values() {
        walk_value(value, parent, rows);
    }
}

fn walk_value(value: &Value, parent: Option<usize>, rows: &mut Vec<Entry>) {
    match value {
        Value::Object(_) => walk(value, parent, rows),
        Value::Array(items) => {
            for item in items {
                walk_value(item, parent, rows);
            }
        }
        _ => {}
    }
}

/// The ruby propagate_scope_ends: a reverse DFS pass settling each
/// scope's effective end as the SMALLER of its own capture line (the
/// seed — which can overshoot into the END keyword + trivia on some
/// grammar shapes) and the maximum effective end of its
/// declaration-level descendants (the ruby lane positions only
/// declaration-level nodes — leaf type/reference/item captures carry
/// no offset there and never raise).
fn propagate(entries: &mut [Entry], seeds: &[usize]) {
    // effective[i] settles in reverse DFS order (children before
    // parents), so a nested scope's final end is available when its
    // parent reads it.
    let mut effective: Vec<usize> = seeds.to_vec();
    for i in (0..entries.len()).rev() {
        let child_end = effective[i];
        if let Some(parent) = entries[i].parent {
            if is_raiser_class(&entries[i].class) && child_end > effective[parent] {
                effective[parent] = child_end;
            }
        }
        if entries[i].is_scope && effective[i] > seeds[i] {
            // The capture seed bounds the scope: overshooting
            // descendants do not extend a scope past its own span.
            effective[i] = seeds[i];
        }
        entries[i].end_line = effective[i];
    }
}

/// The ruby `span_length`: the captured length; the wire always
/// carries source_length on positioned nodes, so end = start + length
/// (identical to source_end_offset here, kept explicit for the
/// contract).
pub fn symbols(source: &str) -> Result<Vec<serde_json::Value>, String> {
    let tree = crate::ParsedTree::parse(source).map_err(|e| format!("parse: {e:?}"))?;
    let wire = tree
        .to_model_json("document.exp")
        .map_err(|e| format!("wire: {e:?}"))?;
    let bytes = source.as_bytes();
    let lines = LineMap::new(bytes);

    let mut entries: Vec<Entry> = Vec::new();
    walk(&wire, None, &mut entries);
    // The ruby grammar lane's capture convention: non-scope nodes
    // carry name/line tokens (the end_line lands on the declaration's
    // first line even when the declaration wraps), and scopes carry a
    // span that ends on their declaration's last line — although some
    // grammar shapes' captures overshoot into the END keyword + trivia
    // the ruby span excludes. Seed from the capture, raise from the
    // declaration-level children, and take the SMALLER of the two for
    // scopes: a scope's true end is its last declaration-level child
    // when the capture overshot, its own span otherwise.
    let seeds: Vec<usize> = entries
        .iter()
        .map(|entry| {
            if entry.is_scope {
                let end = (entry.end as usize).saturating_sub(1);
                lines.line_number(end.max(entry.start as usize))
            } else {
                lines.line_number(entry.start as usize)
            }
        })
        .collect();
    propagate(&mut entries, &seeds);

    let mut rows: Vec<serde_json::Value> = Vec::new();
    for entry in &entries {
        let Some(kind) = symbol_kind(&entry.class) else {
            continue;
        };
        let Some(id) = entry.id.clone() else {
            continue;
        };
        if !entry.valid {
            continue;
        }
        let line = lines.line_number(entry.start as usize);
        if line == 0 {
            continue;
        }
        let sel_start = if entry.is_attribute {
            entry.start as usize
        } else {
            let window_end = ((entry.start as usize) + 1024).min(bytes.len());
            name_offset(
                &bytes[entry.start as usize..window_end],
                entry.start as usize,
                &id,
            )
            .unwrap_or(entry.start as usize)
        };
        rows.push(serde_json::json!({
            "name": id,
            "kind": kind,
            "span_line": line,
            "span_end_line": entry.end_line,
            "sel_start": sel_start,
            "sel_len": id.len(),
        }));
    }
    // The ruby sorts by position, stable to the walk order.
    rows.sort_by_key(|row| row["sel_start"].as_u64().unwrap_or(u64::MAX));
    Ok(rows)
}

/// The delegation entry: document text in, symbol rows out.
pub fn symbols_json(source: &str) -> Result<String, String> {
    let rows = symbols(source)?;
    serde_json::to_string(&rows).map_err(|e| format!("rows json: {e}"))
}

#[cfg(test)]
mod tests {
    use super::symbols_json;

    #[test]
    fn inventories_declarations_with_spans() {
        // `freedoms` wraps onto the next line; the ruby walk's
        // attribute span is the name token, so its end_line stays on
        // the declaration's first line.
        let source = "SCHEMA demo;\n  ENTITY point;\n    freedoms : SET [1:?] OF\n      other_thing;\n  END_ENTITY;\nEND_SCHEMA;\n";
        let rows: Vec<serde_json::Value> =
            serde_json::from_str(&symbols_json(source).expect("rows")).expect("array");
        let names: Vec<&str> = rows
            .iter()
            .map(|row| row["name"].as_str().expect("name"))
            .collect();
        assert_eq!(names, vec!["demo", "point", "freedoms"], "{rows:?}");
        let attr_row = &rows[2];
        assert_eq!(attr_row["span_line"], 3);
        assert_eq!(
            attr_row["span_end_line"], 3,
            "the wrapped attribute's span stays on its first line"
        );
    }
}
