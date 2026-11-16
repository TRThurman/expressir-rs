//! The documentation-coverage engine (#644): per-schema entity counts
//! and documented/undocumented classification over an ATTACHED model
//! wire (the remarks must be attached first — coverage reads
//! `remarks`/`remark_items`). Mirrors the ruby Expressir::Coverage
//! semantics: nested declarations count (attributes, where rules,
//! parameters…), a node's own remarks document it, and the schema's
//! remark_items can document a child by id.

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SchemaCoverage {
    pub schema: String,
    pub total: usize,
    pub documented: usize,
    pub coverage: f64,
    pub undocumented: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CoverageReport {
    pub total: usize,
    pub documented: usize,
    pub undocumented: usize,
    pub coverage: f64,
    pub schemas: Vec<SchemaCoverage>,
}

/// The declaration collections walked per container, with the EXPRESS
/// type name each carries (the skip_types vocabulary).
const DECL_COLLECTIONS: &[(&str, &str)] = &[
    ("types", "TYPE"),
    ("entities", "ENTITY"),
    ("constants", "CONSTANT"),
    ("functions", "FUNCTION"),
    ("rules", "RULE"),
    ("procedures", "PROCEDURE"),
    ("subtype_constraints", "SUBTYPE_CONSTRAINT"),
    ("interfaces", "INTERFACE"),
    ("parameters", "PARAMETER"),
    ("variables", "VARIABLE"),
    ("attributes", "ATTRIBUTE"),
    ("derived_attributes", "DERIVED_ATTRIBUTE"),
    ("inverse_attributes", "INVERSE_ATTRIBUTE"),
    ("unique_rules", "UNIQUE_RULE"),
    ("where_rules", "WHERE_RULE"),
    ("informal_propositions", "INFORMAL_PROPOSITION"),
];

/// Computes documentation coverage over an attached wire.
pub fn coverage(wire: &Value, skip_types: &[String]) -> CoverageReport {
    let mut report = CoverageReport {
        total: 0,
        documented: 0,
        undocumented: 0,
        coverage: 100.0,
        schemas: Vec::new(),
    };

    let Some(schemas) = wire.get("schemas").and_then(Value::as_array) else {
        return report;
    };
    for schema in schemas {
        let schema_id = schema.get("id").and_then(Value::as_str).unwrap_or("");
        let mut entities: Vec<(String, bool)> = Vec::new(); // (id, documented)
        walk_container(schema, skip_types, &mut entities);

        let total = entities.len();
        let documented = entities.iter().filter(|(_, doc)| *doc).count();
        let coverage = if total == 0 {
            100.0
        } else {
            (documented as f64 / total as f64) * 100.0
        };
        let undocumented: Vec<String> = entities
            .iter()
            .filter(|(_, doc)| !doc)
            .map(|(id, _)| format!("{schema_id}.{id}"))
            .collect();

        report.total += total;
        report.documented += documented;
        report.schemas.push(SchemaCoverage {
            schema: schema_id.to_string(),
            total,
            documented,
            coverage,
            undocumented,
        });
    }
    report.undocumented = report.schemas.iter().map(|s| s.undocumented.len()).sum();
    if report.total > 0 {
        report.coverage = (report.documented as f64 / report.total as f64) * 100.0;
    }
    report
}

/// A schema's remark_items document a child by id: either an exact
/// match or the `child.` prefix form (the ruby entity_documented?).
fn schema_documents_child(schema: &Value, id: &str) -> bool {
    let lower = id.to_lowercase();
    schema
        .get("remark_items")
        .and_then(Value::as_array)
        .map(|items| {
            items.iter().any(|item| {
                let item_id = item.get("id").and_then(Value::as_str).unwrap_or("");
                let item_id = item_id.to_lowercase();
                item_id == lower || item_id.starts_with(&format!("{lower}."))
            })
        })
        .unwrap_or(false)
}

fn walk_container(container: &Value, skip_types: &[String], entities: &mut Vec<(String, bool)>) {
    for (key, express_type) in DECL_COLLECTIONS {
        if skip_types.iter().any(|s| s == express_type) {
            continue;
        }
        if let Some(items) = container.get(key).and_then(Value::as_array) {
            for item in items {
                let Some(id) = item.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let documented = has_remarks(item) || schema_documents_child(container, id);
                entities.push((id.to_string(), documented));
                // The nested declarations count too (the ruby
                // find_nested_entities recursion).
                walk_container(item, skip_types, entities);
            }
        }
    }
}

fn has_remarks(node: &Value) -> bool {
    let non_empty_array = |key: &str| {
        node.get(key)
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty())
    };
    non_empty_array("remarks") || non_empty_array("remark_items")
}

#[cfg(test)]
mod tests {
    use super::coverage;
    use serde_json::json;

    #[test]
    fn computes_per_schema_coverage() {
        let wire = json!({
            "_class": "Expressir::Model::ExpFile",
            "schemas": [{
                "_class": "Expressir::Model::Declarations::Schema",
                "id": "demo",
                "remark_items": [],
                "types": [
                    {"_class": "Expressir::Model::Declarations::Type", "id": "colour",
                     "remarks": ["the colour kind"]},
                    {"_class": "Expressir::Model::Declarations::Type", "id": "bare"}
                ],
                "entities": [
                    {"_class": "Expressir::Model::Declarations::Entity", "id": "point",
                     "remarks": ["a point"]},
                    {"_class": "Expressir::Model::Declarations::Entity", "id": "bare_point",
                     "attributes": [
                        {"_class": "Expressir::Model::Declarations::Attribute", "id": "x",
                         "type": {"_class": "Expressir::Model::DataTypes::Real"}}
                     ]}
                ]
            }]
        });
        let report = coverage(&wire, &[]);
        assert_eq!(report.total, 5, "colour+bare+point+bare_point+x");
        assert_eq!(report.documented, 2);
        assert!(report.schemas[0]
            .undocumented
            .contains(&"demo.bare_point".to_string()));
        assert!((report.coverage - 40.0).abs() < 0.01);
    }

    #[test]
    fn schema_remark_items_document_children() {
        let wire = json!({
            "_class": "Expressir::Model::ExpFile",
            "schemas": [{
                "_class": "Expressir::Model::Declarations::Schema",
                "id": "demo",
                "remark_items": [
                    {"_class": "Expressir::Model::Declarations::RemarkItem", "id": "colour",
                     "remarks": ["documented at the schema level"]}
                ],
                "types": [
                    {"_class": "Expressir::Model::Declarations::Type", "id": "colour"}
                ]
            }]
        });
        let report = coverage(&wire, &[]);
        assert_eq!(report.documented, 1);
        assert!((report.coverage - 100.0).abs() < 0.01);
    }

    #[test]
    fn skip_types_filters_collections() {
        let wire = json!({
            "_class": "Expressir::Model::ExpFile",
            "schemas": [{
                "_class": "Expressir::Model::Declarations::Schema",
                "id": "demo",
                "types": [
                    {"_class": "Expressir::Model::Declarations::Type", "id": "colour"}
                ],
                "entities": [
                    {"_class": "Expressir::Model::Declarations::Entity", "id": "point"}
                ]
            }]
        });
        let report = coverage(&wire, &["TYPE".to_string()]);
        assert_eq!(report.total, 1, "only the entity counts");
    }
}
