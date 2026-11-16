//! The clause 9 uos binding writer (#640): a port of the ruby
//! `Expressir::P28::Uos.emit` over the P21::Json population wire and
//! the schema model-hash wire, byte-identical to the ruby lane (the
//! p28_uos_parity gate proves it; the clause ids cite the ruby
//! module's encoding summary).
//!
//! The XML serialization goes through [crate::xml_writer], whose
//! shape was probed against the moxml serializer the ruby lane uses.
//! The delegation seam is `P28::Uos.emit` only: the iso_10303_28
//! document envelope re-serializes parsed units through moxml
//! (namespace-first attribute reordering, whitespace-text
//! preservation), and that transform stays ruby-side.

use std::collections::HashMap;

use serde_json::Value;

use crate::xml_writer::Element;

/// The clause 5 common namespace (Expressir::P28::COMMON_NS).
pub const COMMON_NS: &str = "urn:iso:std:iso:10303:-28:ed-2:tech:XMLschema:common";

/// One schema entity's Part 21 facts: the declaration id and the
/// explicit attribute ids in declaration order.
struct EntityInfo {
    id: String,
    subtype_of: Vec<String>,
    explicit: Vec<String>,
}

/// The schema wire's entities, keyed by downcased id.
struct SchemaIndex {
    entities: HashMap<String, EntityInfo>,
}

impl SchemaIndex {
    fn new(schema: &Value) -> Self {
        let mut entities = HashMap::new();
        if let Some(list) = schema.get("entities").and_then(Value::as_array) {
            for entity in list {
                let Some(id) = entity.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let subtype_of = entity
                    .get("subtype_of")
                    .and_then(Value::as_array)
                    .map(|refs| {
                        refs.iter()
                            .filter_map(|r| r.get("id").and_then(Value::as_str))
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                let explicit = entity
                    .get("attributes")
                    .and_then(Value::as_array)
                    .map(|attrs| {
                        attrs
                            .iter()
                            .filter(|attr| {
                                attr.get("_class")
                                    .and_then(Value::as_str)
                                    .is_some_and(|c| c.ends_with("::Attribute"))
                            })
                            .filter_map(|attr| attr.get("id").and_then(Value::as_str))
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                entities.insert(
                    id.to_lowercase(),
                    EntityInfo {
                        id: id.to_string(),
                        subtype_of,
                        explicit,
                    },
                );
            }
        }
        SchemaIndex { entities }
    }

    /// The ruby supertype_closure order: breadth-first over the
    /// SUBTYPE OF references, self excluded, same-schema names only.
    fn closure(&self, id: &str) -> Vec<&EntityInfo> {
        let mut visited: Vec<String> = vec![id.to_lowercase()];
        let mut queue: std::collections::VecDeque<String> = self
            .entities
            .get(&id.to_lowercase())
            .map(|e| e.subtype_of.iter().map(|s| s.to_lowercase()).collect())
            .unwrap_or_default();
        let mut out = Vec::new();
        while let Some(name) = queue.pop_front() {
            if visited.contains(&name) {
                continue;
            }
            let Some(entity) = self.entities.get(&name) else {
                continue;
            };
            visited.push(name);
            out.push(entity);
            for supertype in &entity.subtype_of {
                queue.push_back(supertype.to_lowercase());
            }
        }
        out
    }

    /// The ruby Eval::Environment#slots_for: inherited explicit
    /// attributes (closure order) then the entity's own, as downcased
    /// slot names indexed by slot position.
    fn slots(&self, id: &str) -> Vec<String> {
        let mut slots: Vec<String> = self
            .closure(id)
            .into_iter()
            .flat_map(|e| {
                e.explicit
                    .iter()
                    .map(|a| a.to_lowercase())
                    .collect::<Vec<_>>()
            })
            .collect();
        if let Some(entity) = self.entities.get(&id.to_lowercase()) {
            slots.extend(entity.explicit.iter().map(|a| a.to_lowercase()));
        }
        slots
    }
}

/// The P21::Json value wire's tagged forms, decoded.
enum P21Value {
    Ref(String),
    Enum(String),
    Binary(String),
    Typed {
        keyword: String,
        value: Vec<P21Value>,
    },
    Unset,
    Derived,
    Str(String),
    Num(String),
    List(Vec<P21Value>),
}

fn decode_value(value: &Value) -> P21Value {
    match value {
        Value::Array(items) => P21Value::List(items.iter().map(decode_value).collect()),
        Value::Object(map) => {
            if let Some(typed) = map.get("typed") {
                P21Value::Typed {
                    keyword: typed["keyword"].as_str().unwrap_or("").to_string(),
                    value: typed
                        .get("value")
                        .and_then(Value::as_array)
                        .map(|items| items.iter().map(decode_value).collect())
                        .unwrap_or_default(),
                }
            } else if let Some(name) = map.get("enum").and_then(Value::as_str) {
                P21Value::Enum(name.to_string())
            } else if let Some(hex) = map.get("binary").and_then(Value::as_str) {
                P21Value::Binary(hex.to_string())
            } else if map.get("unset").and_then(Value::as_bool) == Some(true) {
                P21Value::Unset
            } else if map.get("derived").and_then(Value::as_bool) == Some(true) {
                P21Value::Derived
            } else if let Some(r) = map.get("ref").and_then(Value::as_str) {
                P21Value::Ref(r.to_string())
            } else {
                P21Value::Str(String::new())
            }
        }
        Value::String(s) => {
            if s.starts_with('#') {
                P21Value::Ref(s.clone())
            } else {
                P21Value::Str(s.clone())
            }
        }
        Value::Number(n) => P21Value::Num(ruby_number_text(n)),
        _ => P21Value::Str(String::new()),
    }
}

/// `element_name` (8.5.5): uppercase the first character.
fn element_name(name: &str) -> String {
    let mut out = name.to_string();
    if let Some(first) = out.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    out
}

/// `instance_id`: `#12` -> `id12`.
fn instance_id(pid: &str) -> String {
    format!("id{}", pid.trim_start_matches('#'))
}

/// The 9.5.2/9.5.4 logical face re-encoding; genuine enumeration
/// items pass through as their declared text (9.6.1).
fn logical_text(name: &str) -> String {
    match name.to_ascii_uppercase().as_str() {
        "T" => "true".to_string(),
        "F" => "false".to_string(),
        "UNKNOWN" => "unknown".to_string(),
        _ => name.to_string(),
    }
}

/// Ruby Float#to_s-compatible numeric text (the corpus gate's
/// contract: `100.0` keeps its `.0`; exponents take the ruby
/// mantissa/`e+NN` shape).
fn ruby_number_text(number: &serde_json::Number) -> String {
    if let Some(i) = number.as_i64() {
        return i.to_string();
    }
    if let Some(u) = number.as_u64() {
        return u.to_string();
    }
    let Some(f) = number.as_f64() else {
        return number.to_string();
    };
    if f == 0.0 {
        return if f.is_sign_negative() {
            "-0.0".to_string()
        } else {
            "0.0".to_string()
        };
    }
    let magnitude = f.abs();
    if !(1e-4..1e16).contains(&magnitude) {
        // Ruby: one leading digit, shortest fraction, e+NN/e-NN.
        let shortest = format!("{f:e}");
        let (mantissa, exponent) = shortest.split_once('e').unwrap_or((shortest.as_str(), "0"));
        let exp: i32 = exponent.parse().unwrap_or(0);
        let mantissa = if mantissa.contains('.') {
            mantissa.to_string()
        } else {
            format!("{mantissa}.0")
        };
        return format!(
            "{mantissa}e{}{abs_exp}",
            if exp < 0 { "-" } else { "+" },
            abs_exp = exp.abs()
        );
    }
    let shortest = format!("{f}");
    if shortest.contains('.') || shortest.contains('e') {
        shortest
    } else {
        format!("{shortest}.0")
    }
}

/// The clause 9 uos document for one population wire.
pub fn uos_xml(
    population_wire: &Value,
    schema_wire: &Value,
    id: &str,
    schema_ref: Option<&str>,
    target_namespace: &str,
    prefix: &str,
) -> Result<String, String> {
    let index = SchemaIndex::new(schema_wire);
    let schema_id = schema_wire
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let mut uos = Element::new(format!("{prefix}:uos"))
        .attr("id", id)
        .attr("express", &schema_id)
        .attr(format!("xmlns:{prefix}"), target_namespace)
        .attr("xmlns:exp", crate::p28::COMMON_NS);
    if let Some(r) = schema_ref {
        uos = uos.attr("schema", r);
    }

    let Some(instances) = population_wire.get("instances").and_then(Value::as_object) else {
        return Ok(uos.to_xml());
    };
    for (pid, record) in instances {
        uos = uos.node(instance_element(
            &index,
            prefix,
            population_wire,
            pid,
            record,
        )?);
    }
    Ok(uos.to_xml())
}

fn instance_element(
    index: &SchemaIndex,
    prefix: &str,
    population: &Value,
    pid: &str,
    record: &Value,
) -> Result<Element, String> {
    if record.get("complex").is_some() {
        return complex_entity_element(index, prefix, population, pid, record);
    }
    let keyword = record
        .get("keyword")
        .and_then(Value::as_str)
        .ok_or("instance record missing its keyword")?;
    let entity = index.entities.get(&keyword.to_lowercase());
    let mut node = Element::new(format!(
        "{prefix}:{}",
        instance_name(entity.map(|e| e.id.as_str()), keyword)
    ))
    .attr("id", instance_id(pid));
    let slots = entity.map(|e| index.slots(&e.id)).unwrap_or_default();
    for (i, value) in record
        .get("params")
        .and_then(Value::as_array)
        .map(|params| params.iter().map(decode_value).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .enumerate()
    {
        let name = slots.get(i).cloned();
        if let Some(element) = attribute_value_element(name, &value, population, prefix) {
            node = node.node(element);
        }
    }
    Ok(node)
}

fn complex_entity_element(
    index: &SchemaIndex,
    prefix: &str,
    population: &Value,
    pid: &str,
    record: &Value,
) -> Result<Element, String> {
    let mut node = Element::new("exp:complexEntity").attr("id", instance_id(pid));
    for part in record
        .get("complex")
        .and_then(Value::as_array)
        .ok_or("complex record missing its parts")?
    {
        let keyword = part
            .get("keyword")
            .and_then(Value::as_str)
            .ok_or("complex part missing its keyword")?;
        let entity = index.entities.get(&keyword.to_lowercase());
        let mut child = Element::new(format!(
            "{prefix}:{}",
            instance_name(entity.map(|e| e.id.as_str()), keyword)
        ));
        let slots = entity.map(|e| index.slots(&e.id)).unwrap_or_default();
        for (i, value) in part
            .get("params")
            .and_then(Value::as_array)
            .map(|params| params.iter().map(decode_value).collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .enumerate()
        {
            let name = slots.get(i).cloned();
            if let Some(element) = attribute_value_element(name, &value, population, prefix) {
                child = child.node(element);
            }
        }
        node = node.node(child);
    }
    Ok(node)
}

fn instance_name(entity_id: Option<&str>, keyword: &str) -> String {
    element_name(entity_id.unwrap_or(&keyword.to_lowercase()))
}

/// `Some(element)`, or `None` for the `$`/`*` faces the ruby lane
/// omits (9.3.1: unset and derived parameters emit nothing).
fn attribute_value_element(
    name: Option<String>,
    value: &P21Value,
    population: &Value,
    prefix: &str,
) -> Option<Element> {
    match value {
        P21Value::Unset | P21Value::Derived => None,
        P21Value::List(items) => Some(aggregate_element(name, items, population, prefix)),
        P21Value::Typed { keyword, value } => {
            let mut node = Element::new(element_name(keyword));
            for item in value {
                if let Some(element) = attribute_value_element(None, item, population, prefix) {
                    node = node.node(element);
                }
            }
            Some(node)
        }
        P21Value::Enum(enum_name) => Some(text_element(Some(logical_text(enum_name)), &name)),
        P21Value::Binary(hex) => Some(text_element(Some(hex.clone()), &name)),
        P21Value::Ref(r) => Some(reference_element(name, r)),
        P21Value::Str(text) => Some(text_element(Some(text.clone()), &name)),
        P21Value::Num(text) => Some(text_element(Some(text.clone()), &name)),
    }
}

fn element_for(name: &Option<String>, fallback: &str) -> Element {
    Element::new(match name {
        Some(n) => element_name(n),
        None => fallback.to_string(),
    })
}

fn text_element(text: Option<String>, name: &Option<String>) -> Element {
    let mut node = element_for(name, "Value");
    node = node.text(text.unwrap_or_default());
    node
}

/// 9.3.3: `#n` becomes a by-reference accessor; anything else is a
/// text accessor.
fn reference_element(name: Option<String>, r: &str) -> Element {
    element_for(&name, "Ref").attr("ref", instance_id(r))
}

/// 9.8.1: entity-based aggregates carry one child per component,
/// named by the component's entity type; simple bases are one
/// whitespace LIST text. Complex records have no `keyword`, so their
/// components are `Ref` elements — the ruby lane's `&.keyword` nil
/// path.
fn aggregate_element(
    name: Option<String>,
    values: &[P21Value],
    population: &Value,
    prefix: &str,
) -> Element {
    let mut node = element_for(&name, "List");
    let all_refs = !values.is_empty() && values.iter().all(|v| matches!(v, P21Value::Ref(_)));
    if all_refs {
        for value in values {
            let P21Value::Ref(r) = value else {
                continue;
            };
            let keyword = population["instances"]
                .get(r)
                .and_then(|rec| rec.get("keyword"))
                .and_then(Value::as_str)
                .map(|k| k.to_lowercase())
                .unwrap_or_else(|| "ref".to_string());
            node = node.node(
                Element::new(format!("{}:{}", prefix, element_name(&keyword)))
                    .attr("ref", instance_id(r)),
            );
        }
        return node;
    }
    let text = values.iter().map(scalar_text).collect::<Vec<_>>().join(" ");
    node.text(text)
}

fn scalar_text(value: &P21Value) -> String {
    match value {
        P21Value::Enum(name) => logical_text(name),
        P21Value::Binary(hex) => hex.clone(),
        P21Value::Unset | P21Value::Derived => String::new(),
        P21Value::Str(text) => text.clone(),
        P21Value::Num(text) => text.clone(),
        P21Value::Ref(r) => r.clone(),
        P21Value::List(items) => items.iter().map(scalar_text).collect::<Vec<_>>().join(" "),
        P21Value::Typed { .. } => String::new(),
    }
}

/// The delegation entry: population + schema wires in, the uos
/// document out. `options_json` carries id/schema_ref/target_namespace/
/// prefix exactly like the ruby emit's kwargs.
pub fn uos_xml_json(
    population_json: &str,
    schema_json: &str,
    options_json: &str,
) -> Result<String, String> {
    let population: Value =
        serde_json::from_str(population_json).map_err(|e| format!("population json: {e}"))?;
    let schema: Value =
        serde_json::from_str(schema_json).map_err(|e| format!("schema json: {e}"))?;
    let options: Value =
        serde_json::from_str(options_json).map_err(|e| format!("options json: {e}"))?;
    uos_xml(
        &population,
        &schema,
        options.get("id").and_then(Value::as_str).unwrap_or("uos01"),
        options.get("schema_ref").and_then(Value::as_str),
        options
            .get("target_namespace")
            .and_then(Value::as_str)
            .ok_or("target_namespace is required")?,
        options
            .get("prefix")
            .and_then(Value::as_str)
            .ok_or("prefix is required")?,
    )
}

#[cfg(test)]
mod tests {
    use super::uos_xml_json;
    use serde_json::json;

    /// The clause 9.2.6.1 worked example (the ruby uos_spec's fixture).
    #[test]
    fn emits_the_worked_example() {
        let population = json!({
            "instances": {
                "#10": {"keyword": "PERSON", "params": ["John Q. Public", {"ref": "#20"}]},
                "#20": {"keyword": "CAR", "params": ["BMW", "Z3"]}
            }
        });
        let schema = json!({
            "_class": "Expressir::Model::Declarations::Schema",
            "id": "car_ownership",
            "entities": [
                {"_class": "Expressir::Model::Declarations::Entity", "id": "person",
                 "attributes": [
                    {"_class": "Expressir::Model::Declarations::Attribute", "id": "name"},
                    {"_class": "Expressir::Model::Declarations::Attribute", "id": "owns"}
                 ]},
                {"_class": "Expressir::Model::Declarations::Entity", "id": "car",
                 "attributes": [
                    {"_class": "Expressir::Model::Declarations::Attribute", "id": "make"},
                    {"_class": "Expressir::Model::Declarations::Attribute", "id": "car_model"}
                 ]}
            ]
        });
        let options = json!({"id": "expdata01", "target_namespace": "urn:co", "prefix": "own"});
        let xml = uos_xml_json(
            &serde_json::to_string(&population).unwrap(),
            &serde_json::to_string(&schema).unwrap(),
            &serde_json::to_string(&options).unwrap(),
        )
        .expect("uos xml");
        assert!(xml.starts_with("<own:uos id=\"expdata01\""), "{xml}");
        assert!(xml.contains("<Name>John Q. Public</Name>"), "{xml}");
        assert!(xml.contains("<Owns ref=\"id20\"></Owns>"), "{xml}");
        assert!(xml.contains("<own:Car id=\"id20\">"), "{xml}");
        assert!(xml.contains("<Car_model>Z3</Car_model>"), "{xml}");
    }
}
