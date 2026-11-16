//! The documentation-coverage engine (#644): per-schema entity
//! classification over an ATTACHED model wire (the remarks must be
//! attached first — coverage reads `remarks`/`remark_items`).
//!
//! This is a walk-order mirror of the ruby `Expressir::Coverage`
//! engine: the same containers are visited in the same order, the
//! same classes count, the same skip semantics apply (simple type
//! names, `TYPE:SUBTYPE` via the underlying data type, `FUNCTION:INNER`
//! subtree exclusion applied at class level after collection), and the
//! undocumented entries carry `{type, name}` like the ruby
//! `format_entity`. The differential gate (tests/coverage_parity.rs)
//! proves row-for-row parity against the ruby engine over the corpus;
//! the delegation consumes `rows` zipped with the ruby-side model
//! objects, so the order contract is load-bearing.

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct UndocumentedEntry {
    #[serde(rename = "type")]
    pub express_type: String,
    pub name: String,
}

/// One entity in walk order, pre-filter: `skipped` mirrors the ruby
/// class-level exclusion decision, `documented` mirrors
/// entity_documented?.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EntityRow {
    #[serde(rename = "type")]
    pub express_type: String,
    pub name: String,
    pub documented: bool,
    pub skipped: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SchemaCoverage {
    pub id: String,
    pub total: usize,
    pub documented: usize,
    pub coverage: f64,
    pub rows: Vec<EntityRow>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CoverageReport {
    pub total: usize,
    pub documented: usize,
    pub undocumented: usize,
    pub coverage: f64,
    pub schemas: Vec<SchemaCoverage>,
}

/// Ruby class short name → EXPRESS type name (the
/// CLASS_TO_EXPRESS_TYPE_MAP); anything outside the map falls back to
/// the uppercased short name, exactly like ruby's format_entity.
fn class_to_express(class: &str) -> String {
    let short = class.rsplit("::").next().unwrap_or(class);
    let mapped = match short {
        "Type" => "TYPE",
        "Entity" => "ENTITY",
        "Constant" => "CONSTANT",
        "Function" => "FUNCTION",
        "Rule" => "RULE",
        "Procedure" => "PROCEDURE",
        "SubtypeConstraint" => "SUBTYPE_CONSTRAINT",
        "Parameter" => "PARAMETER",
        "Variable" => "VARIABLE",
        "Attribute" => "ATTRIBUTE",
        "DerivedAttribute" => "DERIVED_ATTRIBUTE",
        "InverseAttribute" => "INVERSE_ATTRIBUTE",
        "UniqueRule" => "UNIQUE_RULE",
        "WhereRule" => "WHERE_RULE",
        "EnumerationItem" => "ENUMERATION_ITEM",
        "Interface" => "INTERFACE",
        "InterfaceItem" => "INTERFACE_ITEM",
        "InterfacedItem" => "INTERFACED_ITEM",
        "SchemaVersion" => "SCHEMA_VERSION",
        "SchemaVersionItem" => "SCHEMA_VERSION_ITEM",
        _ => "",
    };
    if mapped.is_empty() {
        short.to_uppercase()
    } else {
        mapped.to_string()
    }
}

fn class_of(node: &Value) -> &str {
    node.get("_class").and_then(Value::as_str).unwrap_or("")
}

/// The container kinds the ruby find_nested_entities branches over.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Schema,
    Type,
    Entity,
    Scope,
    Interface,
}

fn kind_of(node: &Value) -> Option<Kind> {
    match class_of(node).rsplit("::").next()? {
        "Schema" => Some(Kind::Schema),
        "Type" => Some(Kind::Type),
        "Entity" => Some(Kind::Entity),
        "Function" | "Procedure" | "Rule" => Some(Kind::Scope),
        "Interface" => Some(Kind::Interface),
        _ => None,
    }
}

/// The ruby Schema-level collection order (find_entities_in_schema):
/// declarations first, then the recursion groups.
const SCHEMA_TOP: &[&str] = &[
    "constants",
    "types",
    "entities",
    "functions",
    "rules",
    "procedures",
    "subtype_constraints",
    "interfaces",
];
const SCHEMA_RECURSE: &[&str] = &[
    "types",
    "entities",
    "functions",
    "rules",
    "procedures",
    "interfaces",
];
/// The Function/Procedure/Rule nested_attrs order: the collection
/// attributes minus statements/remark_items/where_rules/informal_
/// propositions/applies_to, in declaration order; then the recursion
/// groups (types, entities, functions, procedures).
const SCOPE_TOP: &[&str] = &[
    "parameters",
    "types",
    "entities",
    "subtype_constraints",
    "functions",
    "procedures",
    "constants",
    "variables",
];
const SCOPE_RECURSE: &[&str] = &["types", "entities", "functions", "procedures"];

/// The skip vocabulary's TYPE subtypes (TYPE_SUBTYPES).
const TYPE_SUBTYPES: &[&str] = &[
    "AGGREGATE",
    "ARRAY",
    "BAG",
    "BINARY",
    "BOOLEAN",
    "ENUMERATION",
    "GENERIC",
    "GENERIC_ENTITY",
    "INTEGER",
    "LIST",
    "LOGICAL",
    "NUMBER",
    "REAL",
    "SELECT",
    "SET",
    "STRING",
];

struct Skips {
    simple: Vec<String>,
    type_subtypes: Vec<String>,
    function_inner: bool,
}

impl Skips {
    fn parse(skip_types: &[String]) -> Self {
        let mut skips = Skips {
            simple: Vec::new(),
            type_subtypes: Vec::new(),
            function_inner: false,
        };
        for skip_type in skip_types {
            if let Some((main, subtype)) = skip_type.split_once(':') {
                if main == "TYPE" && TYPE_SUBTYPES.contains(&subtype) {
                    skips.type_subtypes.push(subtype.to_string());
                } else if main == "FUNCTION" && subtype == "INNER" {
                    skips.function_inner = true;
                }
            } else {
                skips.simple.push(skip_type.clone());
            }
        }
        skips
    }

    fn decides(
        &self,
        row: &EntityRow,
        is_type: bool,
        subtype: Option<&str>,
        within_inner: bool,
    ) -> bool {
        if self.simple.iter().any(|s| s == &row.express_type) {
            return true;
        }
        if is_type
            && !self.type_subtypes.is_empty()
            && subtype.is_some_and(|s| self.type_subtypes.iter().any(|t| t == s))
        {
            return true;
        }
        if self.function_inner && within_inner {
            return true;
        }
        false
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

/// The immediate parent's remark_items document a child by id: the id
/// equal or CONTAINING `id.` (the ruby `include?("#{id}.")` —
/// mid-string matches count).
fn parent_documents(parent: &Value, id: &str) -> bool {
    let lower = id.to_lowercase();
    parent
        .get("remark_items")
        .and_then(Value::as_array)
        .map(|items| {
            items.iter().any(|item| {
                let item_id = item.get("id").and_then(Value::as_str).unwrap_or("");
                let item_id = item_id.to_lowercase();
                item_id == lower || item_id.contains(&format!("{lower}."))
            })
        })
        .unwrap_or(false)
}

/// A TYPE's enumeration items live under the underlying data type on
/// the wire (underlying_type.items when it is an Enumeration); the
/// items' model parent is that data type node.
/// A TYPE's enumeration items live under the underlying data type on
/// the wire — but ONLY for a genuine Enumeration (the ruby
/// `underlying_type.is_a?(DataTypes::Enumeration)` gate; SELECT and
/// the other aggregates also carry `items`, and those are not
/// entities).
fn enumeration_items(type_node: &Value) -> Option<(&Value, Vec<&Value>)> {
    let underlying = type_node.get("underlying_type")?;
    if underlying.get("_class").and_then(Value::as_str)
        != Some("Expressir::Model::DataTypes::Enumeration")
    {
        return None;
    }
    let items = underlying
        .get("items")
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())?;
    Some((underlying, items))
}

fn underlying_subtype(type_node: &Value) -> Option<String> {
    let class = class_of(type_node.get("underlying_type")?);
    let short = class.rsplit("::").next().unwrap_or(class);
    Some(short.to_uppercase())
}

struct Collector {
    skips: Skips,
    rows: Vec<EntityRow>,
}

impl Collector {
    /// Records one entity row in walk order; `parent` is the item's
    /// model parent (the container node, or the underlying data type
    /// for enumeration items).
    fn record(&mut self, item: &Value, parent: &Value, within_inner: bool) {
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            return;
        };
        let class = class_of(item);
        let is_type = class == "Expressir::Model::Declarations::Type";
        let subtype = if is_type {
            underlying_subtype(item)
        } else {
            None
        };
        let row = EntityRow {
            express_type: class_to_express(class),
            name: id.to_string(),
            documented: has_remarks(item) || parent_documents(parent, id),
            skipped: false,
        };
        let skipped = self
            .skips
            .decides(&row, is_type, subtype.as_deref(), within_inner);
        self.rows.push(EntityRow { skipped, ..row });
    }

    /// The ruby find_nested_entities walk: record each kind's
    /// collections in order, then recurse into the recursion groups.
    fn walk(&mut self, node: &Value, kind: Kind, within_inner: bool) {
        match kind {
            Kind::Schema => {
                for key in SCHEMA_TOP {
                    self.record_collection(node, key, within_inner);
                }
                for key in SCHEMA_RECURSE {
                    self.recurse_collection(node, key, within_inner);
                }
            }
            Kind::Scope => {
                for key in SCOPE_TOP {
                    self.record_collection(node, key, within_inner);
                }
                for key in SCOPE_RECURSE {
                    self.recurse_collection(node, key, within_inner);
                }
            }
            Kind::Entity => {
                for key in ["attributes", "unique_rules", "where_rules"] {
                    self.record_collection(node, key, within_inner);
                }
            }
            Kind::Interface => {
                self.record_collection(node, "items", within_inner);
            }
            Kind::Type => {
                // Enumeration items live under the underlying data
                // type, whose node is the items' model parent.
                if let Some((underlying, items)) = enumeration_items(node) {
                    for item in items {
                        self.record(item, underlying, within_inner);
                    }
                }
            }
        }
    }

    fn record_collection(&mut self, node: &Value, key: &str, within_inner: bool) {
        let Some(items) = node.get(key).and_then(Value::as_array) else {
            return;
        };
        for item in items {
            // A function declared inside a Function/Procedure/Rule is
            // an inner function: its row and its whole subtree are
            // excluded from coverage (FUNCTION:INNER / the ruby
            // within_inner_function? ancestor walk).
            let item_inner =
                within_inner || (matches!(kind_of(node), Some(Kind::Scope)) && key == "functions");
            self.record(item, node, item_inner);
        }
    }

    fn recurse_collection(&mut self, node: &Value, key: &str, within_inner: bool) {
        let Some(items) = node.get(key).and_then(Value::as_array) else {
            return;
        };
        for item in items {
            let item_inner =
                within_inner || (matches!(kind_of(node), Some(Kind::Scope)) && key == "functions");
            if let Some(child_kind) = kind_of(item) {
                self.walk(item, child_kind, item_inner);
            }
        }
    }
}

/// Computes documentation coverage over an attached wire. The wire is
/// a repository hash (`files[].schemas[]`, plus any direct `schemas`)
/// or a single ExpFile hash (`schemas`); the schema visit order
/// mirrors Repository#schemas.
pub fn coverage(wire: &Value, skip_types: &[String]) -> CoverageReport {
    let mut schemas_iter: Vec<&Value> = Vec::new();
    if let Some(files) = wire.get("files").and_then(Value::as_array) {
        for file in files {
            if let Some(file_schemas) = file.get("schemas").and_then(Value::as_array) {
                schemas_iter.extend(file_schemas.iter());
            }
        }
    }
    if let Some(direct) = wire.get("schemas").and_then(Value::as_array) {
        schemas_iter.extend(direct.iter());
    }

    let skips = Skips::parse(skip_types);
    let mut report = CoverageReport {
        total: 0,
        documented: 0,
        undocumented: 0,
        coverage: 100.0,
        schemas: Vec::new(),
    };

    for schema in schemas_iter {
        let schema_id = schema.get("id").and_then(Value::as_str).unwrap_or("");
        let mut collector = Collector {
            skips: Skips {
                simple: skips.simple.clone(),
                type_subtypes: skips.type_subtypes.clone(),
                function_inner: skips.function_inner,
            },
            rows: Vec::new(),
        };
        if let Some(kind) = kind_of(schema) {
            collector.walk(schema, kind, false);
        }

        let total = collector.rows.iter().filter(|r| !r.skipped).count();
        let documented = collector
            .rows
            .iter()
            .filter(|r| !r.skipped && r.documented)
            .count();
        let coverage = if total == 0 {
            100.0
        } else {
            (documented as f64 / total as f64) * 100.0
        };

        report.total += total;
        report.documented += documented;
        report.schemas.push(SchemaCoverage {
            id: schema_id.to_string(),
            total,
            documented,
            coverage,
            rows: collector.rows,
        });
    }
    report.undocumented = report
        .schemas
        .iter()
        .map(|s| {
            s.rows
                .iter()
                .filter(|r| !r.skipped && !r.documented)
                .count()
        })
        .sum();
    if report.total > 0 {
        report.coverage = (report.documented as f64 / report.total as f64) * 100.0;
    }
    report
}

/// The delegation entry: wire JSON in, the per-schema row report out
/// (the shape Core.coverage_model_hash hands to the ruby Report).
pub fn coverage_model_json(wire_json: &str, skip_types_json: &str) -> Result<String, String> {
    let wire: Value = serde_json::from_str(wire_json).map_err(|e| format!("wire json: {e}"))?;
    let skip_types: Vec<String> =
        serde_json::from_str(skip_types_json).map_err(|e| format!("skip types json: {e}"))?;
    let report = coverage(&wire, &skip_types);
    serde_json::to_string(&report).map_err(|e| format!("report json: {e}"))
}

#[cfg(test)]
mod tests {
    use super::{coverage, coverage_model_json};
    use serde_json::json;

    #[test]
    fn computes_per_schema_coverage_in_walk_order() {
        let wire = json!({
            "files": [{
                "schemas": [{
                    "_class": "Expressir::Model::Declarations::Schema",
                    "id": "demo",
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
            }]
        });
        let report = coverage(&wire, &[]);
        assert_eq!(report.total, 5, "colour+bare+point+bare_point+x");
        assert_eq!(report.documented, 2);
        let rows = &report.schemas[0].rows;
        let undoc: Vec<&str> = rows
            .iter()
            .filter(|r| !r.skipped && !r.documented)
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(undoc, vec!["bare", "bare_point", "x"], "walk order");
        assert!((report.coverage - 40.0).abs() < 0.01);
    }

    #[test]
    fn enumeration_items_count_as_entities() {
        let wire = json!({
            "schemas": [{
                "_class": "Expressir::Model::Declarations::Schema",
                "id": "demo",
                "types": [{
                    "_class": "Expressir::Model::Declarations::Type", "id": "colour",
                    "underlying_type": {
                        "_class": "Expressir::Model::DataTypes::Enumeration",
                        "items": [
                            {"_class": "Expressir::Model::DataTypes::EnumerationItem",
                             "id": "red", "remarks": ["the red"]},
                            {"_class": "Expressir::Model::DataTypes::EnumerationItem",
                             "id": "blue"}
                        ]
                    }
                }]
            }]
        });
        let report = coverage(&wire, &[]);
        assert_eq!(report.total, 3, "type + two items");
        let rows = &report.schemas[0].rows;
        assert_eq!(rows[1].express_type, "ENUMERATION_ITEM");
        assert!(rows[1].documented);
        assert!(!rows[2].documented);
    }

    #[test]
    fn select_items_are_not_entities() {
        let wire = json!({
            "schemas": [{
                "_class": "Expressir::Model::Declarations::Schema",
                "id": "demo",
                "types": [{
                    "_class": "Expressir::Model::Declarations::Type", "id": "sel",
                    "underlying_type": {
                        "_class": "Expressir::Model::DataTypes::Select",
                        "items": [
                            {"_class": "Expressir::Model::DataTypes::Reference", "id": "point"}
                        ]
                    }
                }]
            }]
        });
        let report = coverage(&wire, &[]);
        assert_eq!(report.total, 1, "only the type counts");
    }

    #[test]
    fn interface_items_count() {
        let wire = json!({
            "schemas": [{
                "_class": "Expressir::Model::Declarations::Schema",
                "id": "demo",
                "interfaces": [{
                    "_class": "Expressir::Model::Declarations::Interface",
                    "kind": "USE",
                    "items": [
                        {"_class": "Expressir::Model::Declarations::InterfaceItem",
                         "id": "base_entity"}
                    ]
                }]
            }]
        });
        let report = coverage(&wire, &[]);
        assert_eq!(report.total, 1, "the interface itself has no id");
        assert_eq!(report.schemas[0].rows[0].express_type, "INTERFACE_ITEM");
    }

    #[test]
    fn schema_remark_items_document_children() {
        let wire = json!({
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
    fn simple_skip_filters_by_class_not_subtree() {
        let wire = json!({
            "schemas": [{
                "_class": "Expressir::Model::Declarations::Schema",
                "id": "demo",
                "entities": [{
                    "_class": "Expressir::Model::Declarations::Entity", "id": "point",
                    "attributes": [
                        {"_class": "Expressir::Model::Declarations::Attribute", "id": "x"}
                    ]
                }]
            }]
        });
        let report = coverage(&wire, &["ENTITY".to_string()]);
        assert_eq!(report.total, 1);
        assert!(report.schemas[0].rows[0].skipped, "the entity drops");
        assert!(!report.schemas[0].rows[1].skipped, "its attribute stays");
    }

    #[test]
    fn type_subtype_skip_reads_the_underlying_type() {
        let wire = json!({
            "schemas": [{
                "_class": "Expressir::Model::Declarations::Schema",
                "id": "demo",
                "types": [
                    {"_class": "Expressir::Model::Declarations::Type", "id": "sel",
                     "underlying_type": {"_class": "Expressir::Model::DataTypes::Select"}},
                    {"_class": "Expressir::Model::Declarations::Type", "id": "num",
                     "underlying_type": {"_class": "Expressir::Model::DataTypes::Real"}}
                ]
            }]
        });
        let report = coverage(&wire, &["TYPE:SELECT".to_string()]);
        assert_eq!(report.total, 1);
        assert!(report.schemas[0].rows[0].skipped, "the select drops");
        assert!(!report.schemas[0].rows[1].skipped, "the real stays");
    }

    #[test]
    fn function_inner_excludes_the_whole_subtree() {
        let wire = json!({
            "schemas": [{
                "_class": "Expressir::Model::Declarations::Schema",
                "id": "demo",
                "functions": [{
                    "_class": "Expressir::Model::Declarations::Function", "id": "outer",
                    "parameters": [
                        {"_class": "Expressir::Model::Declarations::Parameter", "id": "p"}
                    ],
                    "functions": [{
                        "_class": "Expressir::Model::Declarations::Function", "id": "inner",
                        "parameters": [
                            {"_class": "Expressir::Model::Declarations::Parameter", "id": "ip"}
                        ]
                    }]
                }]
            }]
        });
        let all = coverage(&wire, &[]);
        assert_eq!(all.total, 4, "outer+p+inner+ip without skips");

        let report = coverage(&wire, &["FUNCTION:INNER".to_string()]);
        assert_eq!(report.total, 2, "outer + p stay");
        let names: Vec<&str> = report.schemas[0]
            .rows
            .iter()
            .filter(|r| !r.skipped)
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(names, vec!["outer", "p"]);
    }

    #[test]
    fn repository_wire_walks_files_then_direct_schemas() {
        let wire = json!({
            "_class": "Expressir::Model::Repository",
            "files": [
                {"schemas": [{"_class": "Expressir::Model::Declarations::Schema",
                              "id": "a", "entities": [
                    {"_class": "Expressir::Model::Declarations::Entity", "id": "e1"}]}]},
                null,
                {"schemas": [{"_class": "Expressir::Model::Declarations::Schema",
                              "id": "b", "entities": [
                    {"_class": "Expressir::Model::Declarations::Entity", "id": "e2"}]}]}
            ]
        });
        let report = coverage(&wire, &[]);
        assert_eq!(report.schemas.len(), 2, "the failed file is skipped");
        assert_eq!(report.schemas[0].id, "a");
        assert_eq!(report.schemas[1].id, "b");
    }

    #[test]
    fn coverage_model_json_round_shape() {
        let wire = json!({
            "schemas": [{
                "_class": "Expressir::Model::Declarations::Schema",
                "id": "demo",
                "entities": [{"_class": "Expressir::Model::Declarations::Entity",
                              "id": "e1", "remarks": ["doc"]}]
            }]
        });
        let out = coverage_model_json(&serde_json::to_string(&wire).unwrap(), "[]")
            .expect("coverage json");
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed["total"], 1);
        assert_eq!(parsed["schemas"][0]["rows"][0]["type"], "ENTITY");
        assert_eq!(parsed["schemas"][0]["rows"][0]["documented"], true);
    }
}
