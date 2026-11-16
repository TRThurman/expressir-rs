//! The EXPRESS → XML Schema writer (#641): a port of the ruby
//! `Expressir::Express::Xsd.format` over the schema model-hash wire,
//! byte-identical to the ruby lane (the xsd parity gate proves it;
//! expressir #276 step 2). The mapping is the ruby module's v2 doc:
//! entities → element + complexType with a substitutionGroup reference
//! for schema-local supertypes, enumerations → simpleType facets,
//! selects → complexType choice over declared members, defined types
//! → simpleType restriction, built-ins per the ruby table.

use serde_json::Value;

use crate::xml_writer::Element;

const BUILTIN_TYPES: &[(&str, &str)] = &[
    ("string", "xs:string"),
    ("integer", "xs:integer"),
    ("real", "xs:double"),
    ("number", "xs:double"),
    ("boolean", "xs:boolean"),
    ("binary", "xs:hexBinary"),
];

const AGGREGATES: &[&str] = &["array", "list", "set", "bag"];

fn class_of(node: &Value) -> &str {
    node.get("_class").and_then(Value::as_str).unwrap_or("")
}

fn short_name(node: &Value) -> String {
    let class = class_of(node);
    class.rsplit("::").next().unwrap_or(class).to_lowercase()
}

fn builtin(name: &str) -> Option<&'static str> {
    BUILTIN_TYPES
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, value)| *value)
}

fn text_of<'a>(node: &'a Value, key: &str) -> Option<&'a str> {
    node.get(key).and_then(Value::as_str)
}

/// `base_path`'s last segment (the ruby base_id).
fn base_id(base_path: &str) -> &str {
    base_path.rsplit('.').next().unwrap_or(base_path)
}

/// A SimpleReference's resolved id: the base_path tail when the
/// resolution filled one, else the raw id.
fn reference_id(node: &Value) -> Option<&str> {
    text_of(node, "base_path")
        .map(base_id)
        .or_else(|| text_of(node, "id"))
}

/// The schema wire's declared type ids, downcased.
struct SchemaView {
    type_ids: Vec<String>,
    entity_ids: Vec<String>,
}

impl SchemaView {
    fn new(schema: &Value) -> Self {
        SchemaView {
            type_ids: ids_of(schema, "types"),
            entity_ids: ids_of(schema, "entities"),
        }
    }

    fn is_entity(&self, id: &str) -> bool {
        self.entity_ids.iter().any(|e| e == id)
    }

    fn is_type(&self, id: &str) -> bool {
        self.type_ids.iter().any(|t| t == id)
    }
}

fn ids_of(schema: &Value, key: &str) -> Vec<String> {
    schema
        .get(key)
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|n| text_of(n, "id"))
                .map(|id| id.to_lowercase())
                .collect()
        })
        .unwrap_or_default()
}

pub fn xsd_xml(schema_wire: &Value) -> Result<String, String> {
    let view = SchemaView::new(schema_wire);
    let mut root = Element::new("xs:schema")
        .attr("xmlns:xs", "http://www.w3.org/2001/XMLSchema")
        .attr("elementFormDefault", "qualified");
    for type_node in schema_wire
        .get("types")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        root = root.node(type_element(&view, schema_wire, type_node)?);
    }
    for entity in schema_wire
        .get("entities")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        root = root.node(entity_element_node(&view, schema_wire, entity)?);
        root = root.node(entity_type_node(&view, schema_wire, entity)?);
    }
    Ok(root.to_xml())
}

fn type_element(view: &SchemaView, schema: &Value, type_node: &Value) -> Result<Element, String> {
    let underlying = type_node.get("underlying_type");
    if underlying
        .map(|u| short_name(u) == "select")
        .unwrap_or(false)
    {
        let select = underlying.expect("checked above");
        return select_type_node(view, schema, type_node, select);
    }

    let id = text_of(type_node, "id").ok_or("type node missing its id")?;
    let mut node = Element::new("xs:simpleType").attr("name", id);
    let mut restriction = Element::new("xs:restriction");
    let is_enumeration = underlying
        .map(|u| short_name(u) == "enumeration")
        .unwrap_or(false);
    if is_enumeration {
        restriction = restriction.attr("base", "xs:string");
        let enumeration = underlying.expect("checked above");
        for item in enumeration
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let value = text_of(item, "id").unwrap_or("");
            restriction = restriction.node(Element::new("xs:enumeration").attr("value", value));
        }
    } else {
        let base = match underlying {
            Some(u) => xs_type(view, schema, u)?,
            None => "xs:anyType".to_string(),
        };
        restriction = restriction.attr("base", base);
    }
    node = node.node(restriction);
    Ok(node)
}

/// ISO 10303-28: a select type maps to a complexType choosing among
/// its member types. An extensible select with no declared items gets
/// an empty sequence — an empty xs:choice is not schema-valid.
fn select_type_node(
    view: &SchemaView,
    schema: &Value,
    type_node: &Value,
    select: &Value,
) -> Result<Element, String> {
    let id = text_of(type_node, "id").ok_or("type node missing its id")?;
    let items: Vec<&Value> = select
        .get("items")
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    let container = if items.is_empty() {
        "xs:sequence"
    } else {
        "xs:choice"
    };
    let mut node = Element::new("xs:complexType").attr("name", id);
    let mut choice = Element::new(container);
    for item in &items {
        choice = choice.node(select_member_element(view, schema, item)?);
    }
    node = node.node(choice);
    Ok(node)
}

fn select_member_element(
    view: &SchemaView,
    _schema: &Value,
    item: &Value,
) -> Result<Element, String> {
    let mut node = Element::new("xs:element");
    if class_of(item).ends_with("::SimpleReference") {
        let Some(id) = reference_id(item) else {
            return Ok(node);
        };
        if view.is_entity(id) {
            node = node.attr("ref", id);
        } else if view.is_type(id) {
            node = node.attr("name", id).attr("type", id);
        } else {
            node = node.attr("name", id).attr("type", "xs:anyType");
        }
    } else {
        let name = short_name(item);
        let kind = builtin(&name).unwrap_or("xs:anyType");
        node = node.attr("name", &name).attr("type", kind);
    }
    Ok(node)
}

fn entity_element_node(
    view: &SchemaView,
    schema: &Value,
    entity: &Value,
) -> Result<Element, String> {
    let id = text_of(entity, "id").ok_or("entity node missing its id")?;
    let mut node = Element::new("xs:element").attr("name", id).attr("type", id);
    if let Some(parent) = supertype_name(view, schema, entity) {
        node = node.attr("substitutionGroup", parent);
    }
    Ok(node)
}

/// Index of every entity node in the schema wire, by downcased id.
fn entity_index(schema: &Value) -> Vec<(String, usize)> {
    schema
        .get("entities")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .enumerate()
                .filter_map(|(i, n)| text_of(n, "id").map(|id| (id.to_lowercase(), i)))
                .collect()
        })
        .unwrap_or_default()
}

fn entity_at(schema: &Value, index: usize) -> &Value {
    &schema["entities"][index]
}

/// The schema-local supertype NODE indices, in SUBTYPE OF order.
fn local_supertypes(schema: &Value, entity: &Value) -> Vec<usize> {
    let index = entity_index(schema);
    let resolve = |name: &str| {
        let key = name.to_lowercase();
        index.iter().find(|(id, _)| *id == key).map(|(_, i)| *i)
    };
    entity
        .get("subtype_of")
        .and_then(Value::as_array)
        .map(|refs| {
            refs.iter()
                .filter_map(|r| text_of(r, "id"))
                .filter_map(resolve)
                .collect()
        })
        .unwrap_or_default()
}

fn explicit_attribute(attr: &Value) -> bool {
    if class_of(attr).ends_with("::DerivedAttribute") {
        return false;
    }
    text_of(attr, "kind") != Some("INVERSE")
}

fn entity_type_node(view: &SchemaView, schema: &Value, entity: &Value) -> Result<Element, String> {
    let id = text_of(entity, "id").ok_or("entity node missing its id")?;
    let mut sequence = Element::new("xs:sequence");
    for attr in entity
        .get("attributes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if explicit_attribute(attr) {
            sequence = sequence.node(attribute_element(view, schema, attr)?);
        }
    }
    // EXPRESS multiple inheritance: the extension base carries the
    // first parent's chain; later parents' attributes (transitively)
    // become explicit members of the subtype's sequence.
    for attr in additional_parent_attributes(schema, entity) {
        sequence = sequence.node(attribute_element(view, schema, attr)?);
    }

    let mut node = Element::new("xs:complexType").attr("name", id);
    if let Some(base) = supertype_name(view, schema, entity) {
        let extension = Element::new("xs:extension")
            .attr("base", base)
            .node(sequence);
        let content = Element::new("xs:complexContent").node(extension);
        node = node.node(content);
    } else {
        node = node.node(sequence);
    }
    Ok(node)
}

/// Attributes contributed by multiple inheritance: every ancestor
/// reachable from the additional local supertypes (all but the
/// first), minus the first supertype's own chain — the extension
/// base already carries that. Cycle-safe (node-index identity).
fn additional_parent_attributes<'a>(schema: &'a Value, entity: &Value) -> Vec<&'a Value> {
    let parents = local_supertypes(schema, entity);
    if parents.len() <= 1 {
        return Vec::new();
    }

    let mut base_chain = std::collections::HashSet::new();
    let mut queue: std::collections::VecDeque<usize> = [parents[0]].into_iter().collect();
    while let Some(current) = queue.pop_front() {
        if !base_chain.insert(current) {
            continue;
        }
        for supertype in local_supertypes(schema, entity_at(schema, current)) {
            queue.push_back(supertype);
        }
    }

    let mut seen = std::collections::HashSet::new();
    let mut attrs = Vec::new();
    let mut queue: std::collections::VecDeque<usize> = parents[1..].iter().copied().collect();
    while let Some(current) = queue.pop_front() {
        if seen.contains(&current) || base_chain.contains(&current) {
            continue;
        }
        seen.insert(current);
        let node = entity_at(schema, current);
        if let Some(list) = node.get("attributes").and_then(Value::as_array) {
            attrs.extend(list.iter().filter(|a| explicit_attribute(a)));
        }
        for supertype in local_supertypes(schema, node) {
            queue.push_back(supertype);
        }
    }
    attrs
}

fn supertype_name(view: &SchemaView, schema: &Value, entity: &Value) -> Option<String> {
    let _ = view;
    local_supertypes(schema, entity).first().map(|i| {
        text_of(entity_at(schema, *i), "id")
            .unwrap_or("")
            .to_string()
    })
}

fn attribute_element(view: &SchemaView, schema: &Value, attr: &Value) -> Result<Element, String> {
    let name = text_of(attr, "id").unwrap_or("");
    let attr_type = attr.get("type");
    let kind = match attr_type {
        Some(t) => xs_type(view, schema, t)?,
        None => "xs:anyType".to_string(),
    };
    let mut node = Element::new("xs:element")
        .attr("name", name)
        .attr("type", kind);
    if attr_type
        .map(|t| AGGREGATES.contains(&short_name(t).as_str()))
        .unwrap_or(false)
    {
        node = node.attr("maxOccurs", "unbounded");
    }
    Ok(node)
}

fn xs_type(view: &SchemaView, _schema: &Value, type_node: &Value) -> Result<String, String> {
    if class_of(type_node).ends_with("::SimpleReference") {
        if let Some(id) = reference_id(type_node) {
            return Ok(if view.is_type(id) {
                id.to_string()
            } else {
                "xs:anyType".to_string()
            });
        }
        return Ok("xs:anyType".to_string());
    }

    let short = short_name(type_node);
    if short == "enumeration" {
        return Ok(text_of(type_node, "id").unwrap_or("xs:string").to_string());
    }

    // The ruby lane's respond_to?(:base_type): every aggregate kind
    // (aggregate, array, list, set, bag) carries a base_type.
    if matches!(
        short.as_str(),
        "aggregate" | "array" | "list" | "set" | "bag"
    ) {
        if let Some(inner) = type_node.get("base_type") {
            if !inner.is_null() {
                return xs_type(view, _schema, inner);
            }
        }
        return Ok("xs:anyType".to_string());
    }

    Ok(builtin(&short).unwrap_or("xs:anyType").to_string())
}

/// The delegation entry: schema wire in, the xs:schema document out.
pub fn xsd_xml_json(schema_json: &str) -> Result<String, String> {
    let schema: Value =
        serde_json::from_str(schema_json).map_err(|e| format!("schema json: {e}"))?;
    xsd_xml(&schema)
}

#[cfg(test)]
mod tests {
    use super::xsd_xml_json;
    use serde_json::json;

    #[test]
    fn renders_builtin_enum_and_entity() {
        let schema = json!({
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
                    {"_class": "Expressir::Model::Declarations::Attribute", "id": "hue",
                     "type": {"_class": "Expressir::Model::References::SimpleReference", "id": "colour"}}
                 ]}
            ]
        });
        let xml = xsd_xml_json(&serde_json::to_string(&schema).unwrap()).expect("xsd");
        assert!(xml.starts_with("<xs:schema"), "{xml}");
        assert!(
            xml.contains("<xs:enumeration value=\"red\"></xs:enumeration>"),
            "{xml}"
        );
        assert!(
            xml.contains("<xs:element name=\"hue\" type=\"colour\"></xs:element>"),
            "{xml}"
        );
    }
}
