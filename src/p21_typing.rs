//! ISO 10303-11 9.2.5 instance typing (#642 PR-2): which entities a
//! Part 21 instance conforms to. A simple record conforms to its
//! keyword entity plus that entity's supertype closure; a complex
//! record conforms to the union of its records' closures. ONEOF
//! exclusivity (9.2.5.2) is reported per complex record. The schema
//! side reads the #634 model wire.

use serde::Serialize;
use serde_json::Value;

use crate::checker::Ctx;
use crate::population::{FullPopulation, Instance};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TypingResult {
    /// The instance id.
    pub id: u64,
    /// The dotted paths of the entities the instance conforms to.
    pub entities: Vec<String>,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TypingReport {
    pub results: Vec<TypingResult>,
}

/// Types every population instance against a schema wire. `schema`
/// indexes the schema whose declarations the records resolve against
/// (0-based among the wire's schemas).
pub fn type_population(wire: &Value, schema: usize, population: &FullPopulation) -> TypingReport {
    let ctx = Ctx::new(wire);
    let mut results = Vec::new();
    for (id, instance) in &population.instances {
        let parts: Vec<(&str, &str)> = match instance {
            Instance::Simple(record) => vec![("simple", record.keyword.as_str())],
            Instance::Complex(records) => records
                .iter()
                .map(|record| ("complex", record.keyword.as_str()))
                .collect(),
        };
        let mut entities: Vec<String> = Vec::new();
        let mut issues: Vec<String> = Vec::new();
        let mut leaves: Vec<String> = Vec::new();
        let mut seen_leaves: std::collections::HashSet<String> = Default::default();

        for (kind, keyword) in parts {
            let Some(ptr) = find_entity(&ctx, schema, keyword) else {
                issues.push(format!("keyword '{keyword}' is not declared in the schema"));
                continue;
            };
            let entity_id = ctx
                .node(&ptr)
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_lowercase();
            if !seen_leaves.insert(entity_id.clone()) && kind == "complex" {
                issues.push(format!(
                    "complex instance instantiates '{}' twice (9.2.5)",
                    entity_id
                ));
            }
            leaves.push(entity_id.clone());
            entities.push(entity_id.clone());
            for supertype in ctx.supertype_closure(&ptr) {
                if let Some(supertype_id) = ctx.node(&supertype).get("id").and_then(Value::as_str) {
                    entities.push(supertype_id.to_lowercase());
                }
            }
        }

        // ONEOF exclusivity: two leaves sharing a ONEOF group.
        for pair in leaves_combinations(&leaves) {
            if shared_oneof(&ctx, schema, &pair.0, &pair.1) {
                issues.push(format!(
                    "complex instance combines '{}' and '{}' in one ONEOF group (9.2.5.2)",
                    pair.0, pair.1
                ));
            }
        }
        entities.sort();
        entities.dedup();
        results.push(TypingResult {
            id: *id,
            entities,
            issues,
        });
    }
    TypingReport { results }
}

fn leaves_combinations(leaves: &[String]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for i in 0..leaves.len() {
        for j in (i + 1)..leaves.len() {
            out.push((leaves[i].clone(), leaves[j].clone()));
        }
    }
    out
}

/// Resolves a record keyword against a schema's entity declarations
/// (case-insensitive; the checker's children map includes the
/// interface closure).
fn find_entity(ctx: &Ctx<'_>, schema: usize, keyword: &str) -> Option<String> {
    let children = ctx.children_by_id(&ctx.schema_ptr(schema).to_string());
    children.get(&keyword.to_lowercase()).cloned()
}

/// Whether two entities share a ONEOF group: the supertype
/// expressions of their closure members and the ONEOF subtype
/// constraints applied to any of them (9.7).
fn shared_oneof(ctx: &Ctx<'_>, schema: usize, left: &str, right: &str) -> bool {
    let left_groups = oneof_groups(ctx, schema, left);
    let right_groups = oneof_groups(ctx, schema, right);
    left_groups
        .iter()
        .any(|group| right_groups.iter().any(|other| other == group))
}

fn oneof_groups(ctx: &Ctx<'_>, schema: usize, entity_id: &str) -> Vec<Vec<String>> {
    // Resolve the entity name back to its declaration pointer.
    let Some(ptr) = find_entity(ctx, schema, entity_id) else {
        return Vec::new();
    };
    let mut groups: Vec<Vec<String>> = Vec::new();
    let closure = ctx.supertype_closure(&ptr);
    for member in closure {
        let node = ctx.node(&member);
        if let Some(expression) = node.get("supertype_expression") {
            if is_oneof(expression) {
                groups.push(oneof_members(expression));
            }
        }
        // The ONEOF subtype constraints applied to this supertype.
        let supertype_id = node.get("id").and_then(Value::as_str).unwrap_or("");
        for constraint in node.get("subtype_constraints").into_iter() {
            let _ = constraint;
        }
        let base = ctx.schema_ptr(schema).to_string();
        if let Some(list) = ctx
            .schema(schema)
            .get("subtype_constraints")
            .and_then(Value::as_array)
        {
            for constraint in list {
                let applies = constraint
                    .get("applies_to")
                    .and_then(|r| r.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if !applies.eq_ignore_ascii_case(supertype_id) {
                    continue;
                }
                if let Some(expression) = constraint.get("supertype_expression") {
                    if is_oneof(expression) {
                        groups.push(oneof_members(expression));
                    }
                }
            }
        }
        let _ = base;
    }
    groups
}

fn is_oneof(expression: &Value) -> bool {
    expression
        .get("_class")
        .and_then(Value::as_str)
        .is_some_and(|class| class.ends_with("OneofSupertypeExpression"))
}

fn oneof_members(expression: &Value) -> Vec<String> {
    expression
        .get("operands")
        .and_then(Value::as_array)
        .map(|operands| {
            operands
                .iter()
                .filter_map(|operand| {
                    operand
                        .get("id")
                        .and_then(Value::as_str)
                        .map(|id| id.to_lowercase())
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::type_population;
    use crate::population::parse_full_population;
    use serde_json::json;

    fn schema_wire() -> serde_json::Value {
        json!({
            "_class": "Expressir::Model::ExpFile",
            "schemas": [{
                "_class": "Expressir::Model::Declarations::Schema",
                "id": "demo",
                "entities": [
                    {"_class": "Expressir::Model::Declarations::Entity", "id": "base",
                     "supertype_expression": {"_class": "Expressir::Model::SupertypeExpressions::OneofSupertypeExpression",
                        "operands": [
                            {"_class": "Expressir::Model::References::SimpleReference", "id": "sub1"},
                            {"_class": "Expressir::Model::References::SimpleReference", "id": "sub2"}
                        ]}},
                    {"_class": "Expressir::Model::Declarations::Entity", "id": "sub1",
                     "subtype_of": [{"_class": "Expressir::Model::References::SimpleReference", "id": "base"}]},
                    {"_class": "Expressir::Model::Declarations::Entity", "id": "sub2",
                     "subtype_of": [{"_class": "Expressir::Model::References::SimpleReference", "id": "base"}]},
                    {"_class": "Expressir::Model::Declarations::Entity", "id": "leaf"}
                ]
            }]
        })
    }

    #[test]
    fn types_simple_records_with_the_closure() {
        let wire = schema_wire();
        let population = parse_full_population(
            "ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('demo'));\nENDSEC;\nDATA;\n#1=SUB1('x');\n#2=LEAF(1);\nENDSEC;\nEND-ISO-10303-21;\n",
        )
        .expect("parse");
        let report = type_population(&wire, 0, &population);
        let r1 = &report.results[0];
        assert!(r1.issues.is_empty(), "{:?}", r1.issues);
        assert!(r1.entities.contains(&"sub1".to_string()));
        assert!(r1.entities.contains(&"base".to_string()));
        assert_eq!(report.results[1].entities, vec!["leaf".to_string()]);
    }

    #[test]
    fn reports_unknown_keywords_and_oneof_conflicts() {
        let wire = schema_wire();
        let population = parse_full_population(
            "ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('demo'));\nENDSEC;\nDATA;\n#1=(SUB1(1)SUB2(2));\n#3=GHOST(9);\nENDSEC;\nEND-ISO-10303-21;\n",
        )
        .expect("parse");
        let report = type_population(&wire, 0, &population);
        assert!(report.results[0]
            .issues
            .iter()
            .any(|issue| issue.contains("ONEOF")));
        assert!(report.results[1]
            .issues
            .iter()
            .any(|issue| issue.contains("GHOST")));
    }
}
