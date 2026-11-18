//! The remaining export faces (expressir#641): a port of
//! `Expressir::Interop::Owl` (OWL 2 / RDF Turtle), `Interop::JsonSchema`
//! (JSON Schema 2020-12) and `Express::InterfaceDot` (the eeng
//! interface GraphViz emitter) over the model-hash wire. The
//! `*_json` entries are the delegation faces; the parity gate
//! replays the ruby lanes byte-for-byte.

use serde_json::{json, Map, Value};

// ---------------------------------------------------------------------
// InterfaceDot
// ---------------------------------------------------------------------

/// The ruby InterfaceDot#write options.
pub struct DotOptions<'a> {
    pub iface: &'a str,
    pub uf_color: &'a str,
    pub rf_color: &'a str,
    pub depth: Option<usize>,
    pub prune: &'a [String],
    pub version: &'a str,
}

/// The ruby InterfaceDot#write.
pub fn interface_dot(repository: &Value, root_schema: usize, options: &DotOptions) -> String {
    let schemas: Vec<&Value> = repository
        .get("schemas")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter(|s| !s.is_null())
        .collect();
    let by_name: std::collections::HashMap<String, &Value> = schemas
        .iter()
        .filter_map(|s| {
            s.get("id")
                .and_then(Value::as_str)
                .map(|id| (id.to_lowercase(), *s))
        })
        .collect();

    let root_key = schemas
        .get(root_schema)
        .and_then(|s| s.get("id"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase();

    // BFS collect: visited, edges.
    let mut visited: Vec<String> = Vec::new();
    let mut edges: Vec<(String, String, String, bool)> = Vec::new(); // (kind, from, to, items)
    let mut queue: std::collections::VecDeque<(String, usize)> = std::collections::VecDeque::new();
    let mut visited_schemas: Vec<&Value> = Vec::new();
    queue.push_back((root_key.clone(), 0));
    while let Some((key, level)) = queue.pop_front() {
        if visited.contains(&key) || options.prune.contains(&key) {
            continue;
        }
        visited.push(key.clone());
        if let Some(schema) = by_name.get(&key) {
            visited_schemas.push(schema);
        }
        let Some(schema) = by_name.get(&key) else {
            continue;
        };
        for iface_node in schema
            .get("interfaces")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[])
        {
            let Some(target_name) = iface_node
                .get("schema")
                .and_then(|s| {
                    if s.is_string() {
                        s.as_str()
                    } else {
                        s.get("id").and_then(Value::as_str)
                    }
                })
                .map(str::to_string)
            else {
                continue;
            };
            let tkey = target_name.to_lowercase();
            if options.prune.contains(&tkey) || tkey == key {
                continue;
            }
            let class = iface_node
                .get("_class")
                .and_then(Value::as_str)
                .unwrap_or("");
            let kind = if class.ends_with("Interface") {
                let kind_attr = iface_node.get("kind").and_then(Value::as_str).unwrap_or("");
                if kind_attr == "REFERENCE" {
                    "ref"
                } else {
                    "use"
                }
            } else {
                "use"
            };
            if options.iface == "use" && kind != "use" {
                continue;
            }
            if options.iface == "ref" && kind != "ref" {
                continue;
            }
            let items = iface_node
                .get("items")
                .and_then(Value::as_array)
                .is_some_and(|items| !items.is_empty());
            let edge = (kind.to_string(), key.clone(), tkey.clone(), items);
            if !edges.contains(&edge) {
                edges.push(edge);
            }
            if options.depth.is_some_and(|d| level + 1 >= d) {
                continue;
            }
            if by_name.contains_key(&tkey) {
                queue.push_back((tkey, level + 1));
            }
        }
    }

    let mut out = String::new();
    out.push_str("// -*- Mode: Dot -*-\n\n");
    out.push_str("// Dot File Written by Expressir\n");
    out.push_str(&format!("//    {}\n", options.version));
    out.push_str("\ndigraph interfaces {\n");
    out.push_str("  node [shape=none];\n");
    if options.iface == "both" || options.iface == "use" {
        out.push_str(&format!("  edge [color={}];\n", options.uf_color));
        for (kind, from, to, items) in &edges {
            if kind != "use" {
                continue;
            }
            let head = if *items { "curve" } else { "normal" };
            out.push_str(&format!("  {from} -> {to} [arrowhead={head}];\n"));
        }
    }
    if options.iface == "both" || options.iface == "ref" {
        out.push_str(&format!("  edge [color={}];\n", options.rf_color));
        for (kind, from, to, items) in &edges {
            if kind != "ref" {
                continue;
            }
            let head = if *items { "curve" } else { "normal" };
            out.push_str(&format!("  {from} -> {to} [arrowhead={head}];\n"));
        }
    }
    let named: Vec<&String> = edges.iter().flat_map(|(_, f, t, _)| [f, t]).collect();
    for schema in &visited_schemas {
        let id = schema.get("id").and_then(Value::as_str).unwrap_or("");
        let key = id.to_lowercase();
        if !named.contains(&&key) {
            out.push_str(&format!("  {key};\n"));
        }
    }
    out.push_str("}\n");
    out
}

// ---------------------------------------------------------------------
// Interop::Owl
// ---------------------------------------------------------------------

const OWL_PREFIXES: &[(&str, &str)] = &[
    ("owl", "http://www.w3.org/2002/07/owl#"),
    ("rdf", "http://www.w3.org/1999/02/22-rdf-syntax-ns#"),
    ("rdfs", "http://www.w3.org/2000/01/rdf-schema#"),
    ("xsd", "http://www.w3.org/2001/XMLSchema#"),
];

fn primitive_range(class: &str) -> Option<&'static str> {
    match class {
        "Expressir::Model::DataTypes::Integer" => Some("xsd:integer"),
        "Expressir::Model::DataTypes::Real" | "Expressir::Model::DataTypes::Number" => {
            Some("xsd:double")
        }
        "Expressir::Model::DataTypes::String" => Some("xsd:string"),
        "Expressir::Model::DataTypes::Boolean" | "Expressir::Model::DataTypes::Logical" => {
            Some("xsd:boolean")
        }
        "Expressir::Model::DataTypes::Binary" => Some("xsd:hexBinary"),
        _ => None,
    }
}

fn quote(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_default()
}

fn owl_block(predicates: &[String], subject: Option<&str>) -> String {
    let mut lines: Vec<String> = Vec::new();
    match subject {
        Some(subject) => lines.push(format!("{subject} {}", predicates[0])),
        None => lines.push(predicates[0].clone()),
    }
    for line in &predicates[1..] {
        lines.push(format!("    {line}"));
    }
    format!("{} .", lines.join(" ;\n"))
}

fn owl_export(schema: &Value, base_iri: &str) -> String {
    let schema_id = schema.get("id").and_then(Value::as_str).unwrap_or("");
    let mut blocks: Vec<String> = Vec::new();
    blocks.push(owl_block(
        &[
            "a owl:Ontology".to_string(),
            format!("rdfs:label {}", quote(schema_id)),
        ],
        Some(&format!("<{base_iri}>")),
    ));
    for type_node in schema
        .get("types")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        blocks.extend(owl_type_blocks(schema, type_node));
    }
    for entity in schema
        .get("entities")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        blocks.extend(owl_entity_blocks(schema, entity));
    }
    let mut prefixes: Vec<String> = vec![format!("@prefix ex: <{base_iri}> .")];
    prefixes.extend(
        OWL_PREFIXES
            .iter()
            .map(|(name, iri)| format!("@prefix {name}: <{iri}> .")),
    );
    format!("{}\n\n{}", prefixes.join("\n"), blocks.join("\n"))
}

fn owl_entity_blocks(schema: &Value, entity: &Value) -> Vec<String> {
    let id = entity.get("id").and_then(Value::as_str).unwrap_or("");
    let mut lines = vec![
        "a owl:Class".to_string(),
        format!("rdfs:label {}", quote(id)),
    ];
    let supertypes = entity
        .get("subtype_of")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if !supertypes.is_empty() {
        let parents: Vec<String> = supertypes
            .iter()
            .filter_map(|p| p.get("id").and_then(Value::as_str))
            .map(|id| format!("ex:{id}"))
            .collect();
        lines.push(format!("rdfs:subClassOf {}", parents.join(", ")));
    }
    if entity.get("abstract").and_then(Value::as_bool) == Some(true) {
        lines.push(format!(
            "rdfs:comment {}",
            quote("EXPRESS ABSTRACT SUPERTYPE")
        ));
    }
    let mut blocks = vec![owl_block(&lines, Some(&format!("ex:{id}")))];
    for attribute in owl_explicit_attributes(entity) {
        let attribute_type = attribute.get("type").cloned().unwrap_or(Value::Null);
        let range = owl_range(&attribute_type);
        let kind = if owl_object_property(schema, &attribute_type, &mut Vec::new()) {
            "owl:ObjectProperty"
        } else {
            "owl:DatatypeProperty"
        };
        let mut lines = vec![
            format!("a {kind}"),
            format!("rdfs:domain ex:{id}"),
            format!("rdfs:range {range}"),
        ];
        if attribute.get("optional").and_then(Value::as_bool) == Some(true) {
            lines.push(format!("rdfs:comment {}", quote("OPTIONAL")));
        }
        let subject = format!(
            "ex:{}",
            attribute.get("id").and_then(Value::as_str).unwrap_or("")
        );
        blocks.push(owl_block(&lines, Some(&subject)));
    }
    blocks
}

fn owl_explicit_attributes(entity: &Value) -> Vec<&Value> {
    entity
        .get("attributes")
        .and_then(Value::as_array)
        .map(|attrs| {
            attrs
                .iter()
                .filter(|a| {
                    a.get("_class").and_then(Value::as_str)
                        == Some("Expressir::Model::Declarations::Attribute")
                })
                .collect()
        })
        .unwrap_or_default()
}

fn owl_type_blocks(schema: &Value, type_node: &Value) -> Vec<String> {
    let name = type_node.get("id").and_then(Value::as_str).unwrap_or("");
    let underlying = type_node
        .get("underlying_type")
        .cloned()
        .unwrap_or(Value::Null);
    let class = underlying
        .get("_class")
        .and_then(Value::as_str)
        .unwrap_or("");
    match class {
        "Expressir::Model::DataTypes::Enumeration" => {
            owl_enumeration_blocks(schema, name, &underlying, &mut Vec::new())
        }
        "Expressir::Model::DataTypes::Select" => {
            let items = owl_select_items(schema, &underlying, &mut Vec::new());
            let refs: Vec<String> = items.iter().map(|item| format!("ex:{item}")).collect();
            vec![owl_block(
                &[
                    "a owl:Class".to_string(),
                    format!("rdfs:label {}", quote(name)),
                    format!(
                        "owl:equivalentClass [ a owl:Class ; owl:unionOf ( {} ) ]",
                        refs.join(" ")
                    ),
                ],
                Some(&format!("ex:{name}")),
            )]
        }
        _ => vec![owl_defined_type_block(name, &underlying)],
    }
}

fn owl_enumeration_blocks(
    schema: &Value,
    name: &str,
    enumeration: &Value,
    seen: &mut Vec<usize>,
) -> Vec<String> {
    let key = enumeration
        .as_object()
        .map(|o| o as *const _ as usize)
        .unwrap_or(0);
    if seen.contains(&key) {
        return Vec::new();
    }
    seen.push(key);
    let mut blocks = vec![owl_block(
        &[
            "a owl:Class".to_string(),
            format!("rdfs:label {}", quote(name)),
        ],
        Some(&format!("ex:{name}")),
    )];
    for value in owl_enumeration_values(schema, enumeration, seen) {
        blocks.push(format!("ex:{value} a owl:NamedIndividual, ex:{name} ."));
    }
    blocks
}

#[allow(clippy::only_used_in_recursion)]
fn owl_enumeration_values(
    schema: &Value,
    enumeration: &Value,
    seen: &mut Vec<usize>,
) -> Vec<String> {
    let mut values: Vec<String> = Vec::new();
    if let Some(based_on) = enumeration.get("based_on") {
        if let Some(base_type) = owl_resolve(schema, based_on) {
            let underlying = base_type
                .get("underlying_type")
                .cloned()
                .unwrap_or(Value::Null);
            if underlying.get("_class").and_then(Value::as_str)
                == Some("Expressir::Model::DataTypes::Enumeration")
            {
                values.extend(owl_enumeration_values(schema, &underlying, seen));
            }
        }
    }
    if let Some(items) = enumeration.get("items").and_then(Value::as_array) {
        values.extend(
            items
                .iter()
                .filter_map(|i| i.get("id").and_then(Value::as_str))
                .map(str::to_string),
        );
    }
    values.dedup();
    values
}

fn owl_select_items(schema: &Value, select: &Value, seen: &mut Vec<usize>) -> Vec<String> {
    let key = select
        .as_object()
        .map(|o| o as *const _ as usize)
        .unwrap_or(0);
    if seen.contains(&key) {
        return Vec::new();
    }
    seen.push(key);
    let mut names: Vec<String> = Vec::new();
    if let Some(based_on) = select.get("based_on") {
        if let Some(base_type) = owl_resolve(schema, based_on) {
            let underlying = base_type
                .get("underlying_type")
                .cloned()
                .unwrap_or(Value::Null);
            if underlying.get("_class").and_then(Value::as_str)
                == Some("Expressir::Model::DataTypes::Select")
            {
                names.extend(owl_select_items(schema, &underlying, seen));
            }
        }
    }
    if let Some(items) = select.get("items").and_then(Value::as_array) {
        names.extend(
            items
                .iter()
                .filter_map(|i| i.get("id").and_then(Value::as_str))
                .map(str::to_string),
        );
    }
    names.dedup();
    names
}

fn owl_defined_type_block(name: &str, underlying: &Value) -> String {
    let class = underlying
        .get("_class")
        .and_then(Value::as_str)
        .unwrap_or("");
    let primitive = primitive_range(class);
    let mut lines = vec![
        format!(
            "a {}",
            if primitive.is_some() {
                "rdfs:Datatype"
            } else {
                "owl:Class"
            }
        ),
        format!("rdfs:label {}", quote(name)),
    ];
    if let Some(primitive) = primitive {
        lines.push(format!("owl:equivalentClass {primitive}"));
    } else if class == "Expressir::Model::References::SimpleReference" {
        let id = underlying.get("id").and_then(Value::as_str).unwrap_or("");
        lines.push(format!("rdfs:subClassOf ex:{id}"));
    } else {
        lines.push("rdfs:subClassOf rdf:List".to_string());
    }
    owl_block(&lines, Some(&format!("ex:{name}")))
}

fn owl_object_property(schema: &Value, type_node: &Value, seen: &mut Vec<String>) -> bool {
    let class = type_node
        .get("_class")
        .and_then(Value::as_str)
        .unwrap_or("");
    match class {
        "Expressir::Model::DataTypes::List"
        | "Expressir::Model::DataTypes::Array"
        | "Expressir::Model::DataTypes::Set"
        | "Expressir::Model::DataTypes::Bag"
        | "Expressir::Model::DataTypes::Select"
        | "Expressir::Model::DataTypes::Enumeration" => true,
        "Expressir::Model::References::SimpleReference" => {
            let id = type_node.get("id").and_then(Value::as_str).unwrap_or("");
            if seen.iter().any(|s| s == id) {
                return false;
            }
            match owl_resolve(schema, type_node) {
                Some(target) => {
                    let target_class = target.get("_class").and_then(Value::as_str).unwrap_or("");
                    match target_class {
                        "Expressir::Model::Declarations::Entity" => true,
                        "Expressir::Model::Declarations::Type" => {
                            seen.push(id.to_string());
                            let underlying = target
                                .get("underlying_type")
                                .cloned()
                                .unwrap_or(Value::Null);
                            owl_object_property(schema, &underlying, seen)
                        }
                        _ => false,
                    }
                }
                None => false,
            }
        }
        _ => false,
    }
}

fn owl_range(type_node: &Value) -> String {
    let class = type_node
        .get("_class")
        .and_then(Value::as_str)
        .unwrap_or("");
    match class {
        "Expressir::Model::DataTypes::List"
        | "Expressir::Model::DataTypes::Array"
        | "Expressir::Model::DataTypes::Set"
        | "Expressir::Model::DataTypes::Bag" => "rdf:List".to_string(),
        "Expressir::Model::DataTypes::Enumeration" | "Expressir::Model::DataTypes::Select" => {
            "rdfs:Resource".to_string()
        }
        "Expressir::Model::References::SimpleReference" => {
            format!(
                "ex:{}",
                type_node.get("id").and_then(Value::as_str).unwrap_or("")
            )
        }
        _ => primitive_range(class)
            .unwrap_or("rdfs:Resource")
            .to_string(),
    }
}

/// The ruby schema.find: a schema-local entity/type by downcased id.
fn owl_resolve<'a>(schema: &'a Value, reference: &Value) -> Option<&'a Value> {
    let id = reference.get("id").and_then(Value::as_str)?;
    let downcased = id.to_lowercase();
    for collection in ["entities", "types"] {
        let found = schema
            .get(collection)
            .and_then(Value::as_array)
            .and_then(|items| {
                items.iter().find(|item| {
                    item.get("id")
                        .and_then(Value::as_str)
                        .is_some_and(|item_id| item_id.to_lowercase() == downcased)
                })
            });
        if found.is_some() {
            return found;
        }
    }
    None
}

/// The delegation entry: a schema wire + the base IRI (empty = the
/// default) in, the Turtle text out.
pub fn owl_json(schema_json: &str, base_iri: &str) -> Result<String, String> {
    let schema: Value =
        serde_json::from_str(schema_json).map_err(|e| format!("schema json: {e}"))?;
    let id = schema.get("id").and_then(Value::as_str).unwrap_or("");
    let base = if base_iri.is_empty() {
        format!("https://expressir.org/owl/{id}#")
    } else {
        base_iri.to_string()
    };
    Ok(owl_export(&schema, &base))
}

// ---------------------------------------------------------------------
// Interop::JsonSchema
// ---------------------------------------------------------------------

fn js_type_schema(schema: &Value, type_node: &Value, depth: usize) -> Value {
    let class = type_node
        .get("_class")
        .and_then(Value::as_str)
        .unwrap_or("");
    match class {
        "Expressir::Model::DataTypes::Integer" => json!({ "type": "integer" }),
        "Expressir::Model::DataTypes::Real" | "Expressir::Model::DataTypes::Number" => {
            json!({ "type": "number" })
        }
        "Expressir::Model::DataTypes::String" => json!({ "type": "string" }),
        "Expressir::Model::DataTypes::Boolean" => json!({ "type": "boolean" }),
        "Expressir::Model::DataTypes::Logical" => json!({ "enum": [true, false, null] }),
        "Expressir::Model::DataTypes::Binary" => {
            json!({ "type": "string", "pattern": "\\A[01]*\\z" })
        }
        "Expressir::Model::DataTypes::Enumeration" => {
            js_enumeration_schema(schema, type_node, depth)
        }
        "Expressir::Model::DataTypes::Select" => js_select_schema(schema, type_node, depth),
        "Expressir::Model::DataTypes::List"
        | "Expressir::Model::DataTypes::Array"
        | "Expressir::Model::DataTypes::Set"
        | "Expressir::Model::DataTypes::Bag" => js_aggregate_schema(schema, type_node, depth),
        "Expressir::Model::References::SimpleReference" => {
            js_reference(type_node.get("id").and_then(Value::as_str).unwrap_or(""))
        }
        "" => json!({}),
        _ => json!({ "type": "object" }),
    }
}

fn js_reference(id: &str) -> Value {
    json!({ "$ref": format!("#/$defs/{id}") })
}

fn js_enumeration_schema(schema: &Value, enumeration: &Value, depth: usize) -> Value {
    json!({
        "type": "string",
        "enum": js_enumeration_values(schema, enumeration, depth),
    })
}

#[allow(clippy::only_used_in_recursion)]
fn js_enumeration_values(schema: &Value, enumeration: &Value, depth: usize) -> Vec<String> {
    if depth > 32 {
        return Vec::new();
    }
    let mut values: Vec<String> = Vec::new();
    if let Some(based_on) = enumeration.get("based_on") {
        if let Some(base_type) = owl_resolve(schema, based_on) {
            let underlying = base_type
                .get("underlying_type")
                .cloned()
                .unwrap_or(Value::Null);
            if underlying.get("_class").and_then(Value::as_str)
                == Some("Expressir::Model::DataTypes::Enumeration")
            {
                values.extend(js_enumeration_values(schema, &underlying, depth + 1));
            }
        }
    }
    if let Some(items) = enumeration.get("items").and_then(Value::as_array) {
        values.extend(
            items
                .iter()
                .filter_map(|i| i.get("id").and_then(Value::as_str))
                .map(str::to_string),
        );
    }
    values.dedup();
    values
}

fn js_select_schema(schema: &Value, select: &Value, depth: usize) -> Value {
    json!({ "anyOf": js_select_schemas(schema, select, depth) })
}

fn js_select_schemas(schema: &Value, select: &Value, depth: usize) -> Vec<Value> {
    if depth > 32 {
        return Vec::new();
    }
    let mut refs: Vec<Value> = Vec::new();
    if let Some(based_on) = select.get("based_on") {
        if let Some(base_type) = owl_resolve(schema, based_on) {
            let underlying = base_type
                .get("underlying_type")
                .cloned()
                .unwrap_or(Value::Null);
            if underlying.get("_class").and_then(Value::as_str)
                == Some("Expressir::Model::DataTypes::Select")
            {
                refs.extend(js_select_schemas(schema, &underlying, depth + 1));
            }
        }
    }
    if let Some(items) = select.get("items").and_then(Value::as_array) {
        refs.extend(
            items
                .iter()
                .map(|item| js_type_schema(schema, item, depth + 1)),
        );
    }
    refs.dedup();
    refs
}

fn js_aggregate_schema(schema: &Value, type_node: &Value, depth: usize) -> Value {
    let base_type = type_node.get("base_type").cloned().unwrap_or(Value::Null);
    let mut result = json!({
        "type": "array",
        "items": js_type_schema(schema, &base_type, depth + 1),
    });
    let object = result.as_object_mut().expect("object");
    let bound = |key: &str| -> Option<i64> {
        let bound = type_node.get(key)?;
        if bound.get("_class").and_then(Value::as_str)
            == Some("Expressir::Model::Literals::Integer")
        {
            bound
                .get("value")
                .and_then(Value::as_str)
                .and_then(|v| v.parse().ok())
        } else {
            None
        }
    };
    if let Some(lower) = bound("bound1") {
        object.insert("minItems".to_string(), json!(lower));
    }
    if let Some(upper) = bound("bound2") {
        object.insert("maxItems".to_string(), json!(upper));
    }
    let class = type_node
        .get("_class")
        .and_then(Value::as_str)
        .unwrap_or("");
    let unique = class == "Expressir::Model::DataTypes::Set"
        || (class == "Expressir::Model::DataTypes::List"
            && type_node.get("unique").and_then(Value::as_bool) == Some(true));
    if unique {
        object.insert("uniqueItems".to_string(), json!(true));
    }
    result
}

fn js_entity_schema(schema: &Value, entity: &Value, depth: usize) -> Value {
    let id = entity.get("id").and_then(Value::as_str).unwrap_or("");
    let explicit = owl_explicit_attributes(entity);
    let mut properties = Map::new();
    for attribute in &explicit {
        if let Some(attribute_id) = attribute.get("id").and_then(Value::as_str) {
            let attribute_type = attribute.get("type").cloned().unwrap_or(Value::Null);
            let base = js_type_schema(schema, &attribute_type, depth + 1);
            let value = if attribute.get("optional").and_then(Value::as_bool) == Some(true) {
                json!({ "anyOf": [base, { "type": "null" }] })
            } else {
                base
            };
            properties.insert(attribute_id.to_string(), value);
        }
    }
    let mut own = Map::new();
    own.insert("type".to_string(), json!("object"));
    own.insert("properties".to_string(), Value::Object(properties));
    let required: Vec<String> = explicit
        .iter()
        .filter(|a| a.get("optional").and_then(Value::as_bool) != Some(true))
        .filter_map(|a| a.get("id").and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    if !required.is_empty() {
        own.insert("required".to_string(), json!(required));
    }
    let supertypes: Vec<Value> = entity
        .get("subtype_of")
        .and_then(Value::as_array)
        .map(|refs| {
            refs.iter()
                .filter_map(|r| r.get("id").and_then(Value::as_str))
                .map(js_reference)
                .collect()
        })
        .unwrap_or_default();
    if supertypes.is_empty() {
        let mut out = Value::Object(own);
        out["title"] = json!(id);
        out
    } else {
        let mut all_of = supertypes;
        all_of.push(Value::Object(own));
        json!({ "title": id, "allOf": all_of })
    }
}

/// The delegation entry: a schema wire in, the JSON Schema document
/// (as a structure) out — callers pick their own serialization.
pub fn json_schema_json(schema_json: &str) -> Result<String, String> {
    let schema: Value =
        serde_json::from_str(schema_json).map_err(|e| format!("schema json: {e}"))?;
    let schema_id = schema.get("id").and_then(Value::as_str).unwrap_or("");
    let mut definitions = Map::new();
    for type_node in schema
        .get("types")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        let name = type_node.get("id").and_then(Value::as_str).unwrap_or("");
        let underlying = type_node
            .get("underlying_type")
            .cloned()
            .unwrap_or(Value::Null);
        let class = underlying
            .get("_class")
            .and_then(Value::as_str)
            .unwrap_or("");
        let body = match class {
            "Expressir::Model::DataTypes::Enumeration" => {
                js_enumeration_schema(&schema, &underlying, 0)
            }
            "Expressir::Model::DataTypes::Select" => js_select_schema(&schema, &underlying, 0),
            _ => js_type_schema(&schema, &underlying, 0),
        };
        definitions.insert(name.to_string(), body);
    }
    for entity in schema
        .get("entities")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        let name = entity.get("id").and_then(Value::as_str).unwrap_or("");
        definitions.insert(name.to_string(), js_entity_schema(&schema, entity, 0));
    }
    let mut result = Map::new();
    result.insert(
        "$schema".to_string(),
        json!("https://json-schema.org/draft/2020-12/schema"),
    );
    result.insert(
        "$id".to_string(),
        json!(format!("https://expressir.org/json-schema/{schema_id}")),
    );
    result.insert("title".to_string(), json!(schema_id));
    result.insert("$defs".to_string(), Value::Object(definitions));
    let entities: Vec<Value> = schema
        .get("entities")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|e| e.get("id").and_then(Value::as_str))
                .map(js_reference)
                .collect()
        })
        .unwrap_or_default();
    if !entities.is_empty() {
        result.insert("anyOf".to_string(), json!(entities));
    }
    serde_json::to_string(&Value::Object(result)).map_err(|e| format!("result json: {e}"))
}
