//! The ISO 10303-21 semantic validator (#642 PR-2d): header/section
//! conformance, clause-12 record checks (unknown entities, clique,
//! arity, derived slots, unknown references, enum membership) and the
//! TODO.instance/01 value/type conformance (primitive domains,
//! aggregate bounds, reference typing). The WHERE-rule domain checks
//! need the expression evaluator (#639) and are reported by the ruby
//! lane only for now.

use serde::Serialize;
use serde_json::Value;

use crate::checker::Ctx;
use crate::population::{FullPopulation, Instance, P21Value, Record};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Note {
    pub id: String,
    pub severity: String,
    pub instance: Option<u64>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ValidationReport {
    pub notes: Vec<Note>,
}

impl ValidationReport {
    pub fn valid(&self) -> bool {
        self.notes.iter().all(|n| n.severity != "error")
    }
}

struct Validator<'a> {
    ctx: Ctx<'a>,
    schema: usize,
    population: &'a FullPopulation,
    notes: Vec<Note>,
}

pub fn validate(wire: &Value, schema: usize, population: &FullPopulation) -> ValidationReport {
    let mut validator = Validator {
        ctx: Ctx::new(wire),
        schema,
        population,
        notes: Vec::new(),
    };
    validator.run();
    ValidationReport {
        notes: validator.notes,
    }
}

impl<'a> Validator<'a> {
    fn run(&mut self) {
        self.check_file_schema();
        self.check_sections();
        let ids: Vec<u64> = self
            .population
            .instances
            .iter()
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            if let Some((_, instance)) =
                self.population.instances.iter().find(|(key, _)| *key == id)
            {
                let instance = instance.clone();
                self.check_instance(id, &instance);
            }
        }
    }

    fn schema_id(&self) -> String {
        self.ctx
            .schema(self.schema)
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    }

    fn header_file_schema(&self) -> Option<String> {
        let entry = self
            .population
            .header
            .iter()
            .find(|(keyword, _)| keyword.eq_ignore_ascii_case("FILE_SCHEMA"))?;
        let list = entry.1.first()?;
        match list {
            P21Value::List(items) => items.first(),
            other => Some(other),
        }
        .and_then(|value| match value {
            P21Value::Str(text) => Some(text.clone()),
            _ => None,
        })
    }

    fn check_file_schema(&mut self) {
        let Some(declared) = self.header_file_schema() else {
            return;
        };
        let schema_id = self.schema_id();
        if schema_id.is_empty() || declared.eq_ignore_ascii_case(&schema_id) {
            return;
        }
        self.notes.push(Note {
            id: "p21_schema_mismatch".into(),
            severity: "warning".into(),
            instance: None,
            message: format!("FILE_SCHEMA '{declared}' does not match schema '{schema_id}'"),
        });
    }

    fn check_sections(&mut self) {
        let mut known = vec![self.schema_id().to_lowercase()];
        if let Some(declared) = self.header_file_schema() {
            known.push(declared.to_lowercase());
        }
        let sections = self.population.sections.clone();
        for section in &sections {
            if section.contexts.is_empty() {
                continue;
            }
            if section
                .contexts
                .iter()
                .any(|context| known.contains(&context.to_lowercase()))
            {
                continue;
            }
            self.notes.push(Note {
                id: "p21_section_context".into(),
                severity: "warning".into(),
                instance: None,
                message: format!(
                    "DATA section '{}' context(s) {} are not in the schema list",
                    section.name.clone().unwrap_or_default(),
                    section.contexts.join(", ")
                ),
            });
        }
    }

    fn find_entity(&self, keyword: &str) -> Option<String> {
        let children = self.ctx.children_by_id(self.ctx.schema_ptr(self.schema));
        children
            .get(&keyword.to_lowercase())
            .filter(|ptr| self.ctx.node(ptr).get("attributes").is_some())
            .cloned()
    }

    fn find_type(&self, keyword: &str) -> Option<String> {
        let children = self.ctx.children_by_id(self.ctx.schema_ptr(self.schema));
        children
            .get(&keyword.to_lowercase())
            .filter(|ptr| {
                let class = self
                    .ctx
                    .node(ptr)
                    .get("_class")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                class.ends_with("::Type")
            })
            .cloned()
    }

    fn check_instance(&mut self, id: u64, instance: &Instance) {
        match instance {
            Instance::Complex(records) => {
                let mut pairs = Vec::new();
                for record in records {
                    match self.find_entity(&record.keyword) {
                        Some(ptr) => pairs.push((record, ptr)),
                        None => self.note(
                            "p21_unknown_entity",
                            "error",
                            id,
                            format!(
                                "{} is not declared in schema '{}'",
                                record.keyword,
                                self.schema_id()
                            ),
                        ),
                    }
                }
                if pairs.is_empty() {
                    return;
                }
                self.check_clique(id, &pairs);
                for (record, ptr) in &pairs {
                    let slots = self.mapped_slots(ptr);
                    self.check_params(id, &record.params, &slots);
                }
            }
            Instance::Simple(record) => {
                let Some(ptr) = self.find_entity(&record.keyword) else {
                    self.note(
                        "p21_unknown_entity",
                        "error",
                        id,
                        format!(
                            "{} is not declared in schema '{}'",
                            record.keyword,
                            self.schema_id()
                        ),
                    );
                    return;
                };
                let slots = self.mapped_slots(&ptr);
                self.check_params(id, &record.params, &slots);
            }
        }
    }

    fn check_clique(&mut self, id: u64, pairs: &[(&Record, String)]) {
        if pairs.len() < 2 {
            return;
        }
        let mut common: Vec<String> = self.closure_keys(&pairs[0].1);
        for (_, ptr) in pairs.iter().skip(1) {
            let keys = self.closure_keys(ptr);
            common.retain(|key| keys.contains(key));
        }
        if common.is_empty() {
            self.note(
                "p21_complex_clique",
                "error",
                id,
                format!("{id}: complex records share no common supertype"),
            );
        }
    }

    fn closure_keys(&self, ptr: &str) -> Vec<String> {
        let mut keys: Vec<String> = self
            .ctx
            .supertype_closure(ptr)
            .into_iter()
            .map(|member| {
                self.ctx
                    .node(&member)
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_lowercase()
            })
            .collect();
        keys.push(
            self.ctx
                .node(ptr)
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_lowercase(),
        );
        keys
    }

    /// The Part 21 value order: supertype attributes first; an
    /// attribute redeclared DERIVED in the entity keeps its inherited
    /// slot, valued `*` (12.2.6).
    fn mapped_slots(&self, ptr: &str) -> Vec<(Value, bool)> {
        let own_derived: Vec<String> = self
            .ctx
            .node(ptr)
            .get("attributes")
            .and_then(Value::as_array)
            .map(|attrs| {
                attrs
                    .iter()
                    .filter(|attr| {
                        attr.get("_class")
                            .and_then(Value::as_str)
                            .is_some_and(|c| c.ends_with("DerivedAttribute"))
                    })
                    .filter_map(|attr| attr.get("id").and_then(Value::as_str))
                    .map(|id| id.to_lowercase())
                    .collect()
            })
            .unwrap_or_default();
        let mut slots = Vec::new();
        for supertype in self.ctx.supertype_closure(ptr) {
            if let Some(attrs) = self
                .ctx
                .node(&supertype)
                .get("attributes")
                .and_then(Value::as_array)
            {
                for attr in attrs {
                    if self.is_explicit(attr) {
                        let derived_slot = attr
                            .get("id")
                            .and_then(Value::as_str)
                            .map(|id| own_derived.contains(&id.to_lowercase()))
                            .unwrap_or(false);
                        slots.push((attr.clone(), derived_slot));
                    }
                }
            }
        }
        if let Some(attrs) = self
            .ctx
            .node(ptr)
            .get("attributes")
            .and_then(Value::as_array)
        {
            for attr in attrs {
                if self.is_explicit(attr) {
                    slots.push((attr.clone(), false));
                }
            }
        }
        slots
    }

    fn is_explicit(&self, attr: &Value) -> bool {
        attr.get("_class")
            .and_then(Value::as_str)
            .is_some_and(|c| c.ends_with("::Attribute"))
    }

    fn check_params(&mut self, id: u64, params: &[P21Value], slots: &[(Value, bool)]) {
        if params.len() != slots.len() {
            self.note(
                "p21_arity_mismatch",
                "error",
                id,
                format!(
                    "{id}: {} values for {} mapped attributes",
                    params.len(),
                    slots.len()
                ),
            );
        }
        for ((attr, derived_slot), value) in slots.iter().zip(params) {
            if matches!(value, P21Value::Unset) {
                continue;
            }
            if *derived_slot {
                if !matches!(value, P21Value::Derived) {
                    let attr_id = attr.get("id").and_then(Value::as_str).unwrap_or("?");
                    self.note(
                        "p21_derived_slot",
                        "error",
                        id,
                        format!("{id}: attribute '{attr_id}' redeclared DERIVED must be '*'"),
                    );
                }
                continue;
            }
            self.check_value(id, attr, value);
        }
    }

    fn check_value(&mut self, id: u64, attr: &Value, value: &P21Value) {
        match value {
            P21Value::Ref(target) => {
                let key = format!("#{target}");
                if !self
                    .population
                    .instances
                    .iter()
                    .any(|(known, _)| format!("#{known}") == key)
                    && !self
                        .population
                        .references
                        .iter()
                        .any(|(known, _)| format!("#{known}") == key)
                {
                    self.note(
                        "p21_unknown_reference",
                        "error",
                        id,
                        format!("{id}: reference {key} has no instance"),
                    );
                }
            }
            P21Value::Derived => {
                self.note(
                    "p21_derived_value",
                    "error",
                    id,
                    format!("{id}: '*' outside a derived slot"),
                );
            }
            P21Value::Typed(typed) => {
                if self.find_entity(&typed.keyword).is_none()
                    && self.find_type(&typed.keyword).is_none()
                {
                    self.note(
                        "p21_unknown_typed_keyword",
                        "error",
                        id,
                        format!("{id}: typed parameter '{}' does not resolve", typed.keyword),
                    );
                }
            }
            P21Value::Enum(name) => {
                if let Some(items) = self.enumeration_items(attr) {
                    if !items.contains(&name.to_lowercase()) {
                        let attr_id = attr.get("id").and_then(Value::as_str).unwrap_or("?");
                        self.note(
                            "p21_enum_value",
                            "error",
                            id,
                            format!("{id}: .{name}. is not in attribute '{attr_id}' enumeration"),
                        );
                    }
                }
            }
            P21Value::List(items) => {
                for item in items {
                    self.check_value(id, attr, item);
                }
            }
            _ => {}
        }
    }

    fn enumeration_items(&self, attr: &Value) -> Option<Vec<String>> {
        let type_node = attr.get("type")?;
        let class = type_node.get("_class").and_then(Value::as_str)?;
        if class.ends_with("::Enumeration") {
            return Some(Self::enum_names(type_node));
        }
        if !class.ends_with("::SimpleReference") {
            return None;
        }
        let type_id = type_node.get("id").and_then(Value::as_str)?;
        let declaration = self.find_type(type_id)?;
        let underlying = self.ctx.node(&declaration).get("underlying_type")?;
        let class = underlying.get("_class").and_then(Value::as_str)?;
        if class.ends_with("::Enumeration") {
            Some(Self::enum_names(underlying))
        } else {
            None
        }
    }

    fn enum_names(type_node: &Value) -> Vec<String> {
        type_node
            .get("items")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("id").and_then(Value::as_str))
                    .map(|id| id.to_lowercase())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn note(&mut self, id: &str, severity: &str, instance: u64, message: String) {
        self.notes.push(Note {
            id: id.to_string(),
            severity: severity.to_string(),
            instance: Some(instance),
            message,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::validate;
    use crate::population::parse_full_population;
    use serde_json::json;

    fn wire() -> serde_json::Value {
        json!({
            "_class": "Expressir::Model::ExpFile",
            "schemas": [{
                "_class": "Expressir::Model::Declarations::Schema",
                "id": "demo",
                "types": [
                    {"_class": "Expressir::Model::Declarations::Type", "id": "colour",
                     "underlying_type": {"_class": "Expressir::Model::DataTypes::Enumeration",
                        "items": [
                            {"_class": "Expressir::Model::DataTypes::EnumerationItem", "id": "red"},
                            {"_class": "Expressir::Model::DataTypes::EnumerationItem", "id": "blue"}
                        ]}}
                ],
                "entities": [
                    {"_class": "Expressir::Model::Declarations::Entity", "id": "point",
                     "attributes": [
                        {"_class": "Expressir::Model::Declarations::Attribute", "id": "x",
                         "type": {"_class": "Expressir::Model::DataTypes::Real"}},
                        {"_class": "Expressir::Model::Declarations::Attribute", "id": "c",
                         "type": {"_class": "Expressir::Model::References::SimpleReference", "id": "colour"}}
                    ]},
                    {"_class": "Expressir::Model::Declarations::Entity", "id": "ghost_holder",
                     "attributes": [
                        {"_class": "Expressir::Model::Declarations::Attribute", "id": "p",
                         "type": {"_class": "Expressir::Model::References::SimpleReference", "id": "point"}}
                    ]}
                ]
            }]
        })
    }

    #[test]
    fn flags_schema_mismatch_and_unknown_entity() {
        let population = parse_full_population(
            "ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('other'));\nENDSEC;\nDATA;\n#1=NOPE(1,2);\nENDSEC;\nEND-ISO-10303-21;\n",
        )
        .expect("parse");
        let report = validate(&wire(), 0, &population);
        assert!(report
            .notes
            .iter()
            .any(|n| n.id == "p21_schema_mismatch" && n.severity == "warning"));
        assert!(report
            .notes
            .iter()
            .any(|n| n.id == "p21_unknown_entity" && n.severity == "error"));
        assert!(!report.valid());
    }

    #[test]
    fn flags_arity_and_enum_and_unknown_reference() {
        // #2 has 1 value for 2 slots; its enum .green. is not in colour;
        // #3 references a missing instance.
        let population = parse_full_population(
            "ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('demo'));\nENDSEC;\nDATA;\n#2=POINT(1);\n#3=GHOST_HOLDER(#99);\nENDSEC;\nEND-ISO-10303-21;\n",
        )
        .expect("parse");
        let report = validate(&wire(), 0, &population);
        for expected in ["p21_arity_mismatch", "p21_unknown_reference"] {
            assert!(
                report.notes.iter().any(|n| n.id == expected),
                "missing {expected}: {:?}",
                report.notes
            );
        }
    }

    #[test]
    fn accepts_a_conforming_instance() {
        let population = parse_full_population(
            "ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('demo'));\nENDSEC;\nDATA;\n#1=POINT(1.5,.BLUE.);\n#2=GHOST_HOLDER(#1);\nENDSEC;\nEND-ISO-10303-21;\n",
        )
        .expect("parse");
        let report = validate(&wire(), 0, &population);
        assert!(
            report.notes.is_empty(),
            "unexpected notes: {:?}",
            report.notes
        );
        assert!(report.valid());
    }
}
