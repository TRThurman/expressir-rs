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

// ---------------------------------------------------------------------
// The reader faces (expressir#640): the minimal XML reader for the
// iso_10303_28/uos document subset, then Uos.parse and Document.parse
// — ports of the ruby P28::Uos.parse / P28::Document.parse. The ruby
// tier keeps moxml+leptris; this reader serves the rs core lane.
// ---------------------------------------------------------------------

/// A parsed element: name, attributes in document order, children,
/// and the character data of the whole subtree (the ruby node.text).
#[derive(Debug, Clone)]
pub struct XNode {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<XNode>,
    text: String,
}

impl XNode {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// The ruby node.text: every descendant character data node,
    /// concatenated in document order.
    fn text(&self) -> &str {
        &self.text
    }

    fn elements(&self) -> impl Iterator<Item = &XNode> {
        self.children.iter()
    }
}

fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some(semi) = text[i..].find(';') {
                let entity = &text[i + 1..i + semi];
                let replacement = match entity {
                    "lt" => Some("<".to_string()),
                    "gt" => Some(">".to_string()),
                    "amp" => Some("&".to_string()),
                    "quot" => Some("\"".to_string()),
                    "apos" => Some("'".to_string()),
                    other => other
                        .strip_prefix('#')
                        .and_then(|digits| {
                            if let Some(hex) = digits
                                .strip_prefix('x')
                                .or_else(|| digits.strip_prefix('X'))
                            {
                                u32::from_str_radix(hex, 16).ok()
                            } else {
                                digits.parse::<u32>().ok()
                            }
                        })
                        .and_then(char::from_u32)
                        .map(|c| c.to_string()),
                };
                if let Some(text) = replacement {
                    out.push_str(&text);
                    i += semi + 1;
                    continue;
                }
            }
        }
        let ch = text[i..].chars().next().unwrap_or('&');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

struct XmlReader<'a> {
    chars: std::str::CharIndices<'a>,
    peeked: Option<(usize, char)>,
    src: &'a str,
}

impl<'a> XmlReader<'a> {
    fn new(src: &'a str) -> Self {
        XmlReader {
            chars: src.char_indices(),
            peeked: None,
            src,
        }
    }

    fn next_char(&mut self) -> Option<(usize, char)> {
        if self.peeked.is_some() {
            return self.peeked.take();
        }
        self.chars.next()
    }

    fn peek_char(&mut self) -> Option<(usize, char)> {
        if self.peeked.is_none() {
            self.peeked = self.chars.next();
        }
        self.peeked
    }

    fn slice(&self, start: usize, end: usize) -> &'a str {
        &self.src[start..end]
    }

    fn skip_until(&mut self, needle: &str) {
        if let Some((start, _)) = self.peek_char() {
            if let Some(pos) = self.src[start..].find(needle) {
                let target = start + pos + needle.len();
                while let Some((i, _)) = self.next_char() {
                    if i + 1 >= target {
                        break;
                    }
                }
            } else {
                for _ in self.by_ref() {}
            }
        }
    }
}

impl Iterator for XmlReader<'_> {
    type Item = (usize, char);
    fn next(&mut self) -> Option<Self::Item> {
        self.next_char()
    }
}

/// The ruby lane reads moxml's LOCAL names — "ep:Node" reads as
/// "Node". Strip any prefix at parse time.
fn local_name(raw: &str) -> String {
    raw.rsplit(':').next().unwrap_or(raw).to_string()
}

fn parse_xml(source: &str) -> Result<XNode, String> {
    let mut reader = XmlReader::new(source);
    let mut stack: Vec<XNode> = Vec::new();
    let mut root: Option<XNode> = None;
    let mut text_start: Option<usize> = None;
    while let Some((pos, ch)) = reader.next_char() {
        if ch == '<' {
            if let Some(start) = text_start.take() {
                let raw = reader.slice(start, pos);
                let decoded = decode_entities(raw);
                if !decoded.is_empty() {
                    if let Some(top) = stack.last_mut() {
                        top.text.push_str(&decoded);
                    }
                }
            }
            match reader.peek_char() {
                Some((_, '!')) if reader.src[pos..].starts_with("<!--") => {
                    reader.skip_until("-->");
                }
                Some((_, '!')) | Some((_, '?')) => reader.skip_until(">"),
                Some((_, '/')) => {
                    // closing tag
                    reader.next_char();
                    let mut name = String::new();
                    while let Some((_, c)) = reader.next_char() {
                        if c == '>' {
                            break;
                        }
                        name.push(c);
                    }
                    let name = local_name(name.trim());
                    let Some(top) = stack.pop() else {
                        return Err(format!("unexpected closing tag </{name}>"));
                    };
                    if top.name != name {
                        return Err(format!(
                            "mismatched closing tag </{name}> for <{}>",
                            top.name
                        ));
                    }
                    if let Some(parent) = stack.last_mut() {
                        parent.text.push_str(&top.text);
                        parent.children.push(top);
                    } else {
                        root = Some(top);
                    }
                }
                _ => {
                    // opening tag: read the name
                    let mut name = String::new();
                    let mut self_closing = false;
                    loop {
                        let Some((_, c)) = reader.next_char() else {
                            return Err("unterminated tag".to_string());
                        };
                        if c == '>' {
                            break;
                        }
                        if c == '/' && reader.peek_char().is_some_and(|(_, nc)| nc == '>') {
                            self_closing = true;
                            continue;
                        }
                        if name.is_empty() && c.is_whitespace() {
                            continue;
                        }
                        if c.is_whitespace() && !name.is_empty() {
                            // move to attributes
                            let attrs = read_attrs(&mut reader)?;
                            if reader.peek_char().is_some_and(|(_, nc)| nc == '/') {
                                reader.next_char();
                                self_closing = true;
                            }
                            // consume the closing '>'
                            loop {
                                let Some((_, c)) = reader.next_char() else {
                                    return Err("unterminated tag".to_string());
                                };
                                if c == '>' {
                                    break;
                                }
                            }
                            let node = XNode {
                                name: local_name(&name),
                                attrs,
                                children: Vec::new(),
                                text: String::new(),
                            };
                            if self_closing {
                                if let Some(parent) = stack.last_mut() {
                                    parent.children.push(node);
                                } else {
                                    root = Some(node);
                                }
                            } else {
                                stack.push(node);
                            }
                            name.clear();
                            break;
                        }
                        name.push(c);
                    }
                    if !name.is_empty() {
                        // no attributes: consumed straight to '>'
                        let node = XNode {
                            name: local_name(&name),
                            attrs: Vec::new(),
                            children: Vec::new(),
                            text: String::new(),
                        };
                        if self_closing {
                            if let Some(parent) = stack.last_mut() {
                                parent.children.push(node);
                            } else {
                                root = Some(node);
                            }
                        } else {
                            stack.push(node);
                        }
                    }
                    text_start = Some(pos + 1 + name.len());
                    // text_start fixed up after attr consumption below
                    if let Some((next_pos, _)) = reader.peek_char() {
                        text_start = Some(next_pos);
                    }
                }
            }
        }
    }
    match root {
        Some(root) => Ok(root),
        None => Err("no root element".to_string()),
    }
}

fn read_attrs(reader: &mut XmlReader) -> Result<Vec<(String, String)>, String> {
    let mut attrs: Vec<(String, String)> = Vec::new();
    loop {
        // skip whitespace
        let Some((_, c)) = reader.peek_char() else {
            return Err("unterminated tag".to_string());
        };
        if c == '>' || c == '/' {
            return Ok(attrs);
        }
        if c.is_whitespace() {
            reader.next_char();
            continue;
        }
        let mut name = String::new();
        while let Some((_, ch)) = reader.peek_char() {
            if ch.is_whitespace() || ch == '=' || ch == '>' || ch == '/' {
                break;
            }
            name.push(ch);
            reader.next_char();
        }
        // skip whitespace before '='
        while reader.peek_char().is_some_and(|(_, ch)| ch.is_whitespace()) {
            reader.next_char();
        }
        if reader.peek_char().is_some_and(|(_, ch)| ch == '=') {
            reader.next_char();
            while reader.peek_char().is_some_and(|(_, ch)| ch.is_whitespace()) {
                reader.next_char();
            }
            let Some((_, quote)) = reader.next_char() else {
                return Err("unterminated attribute".to_string());
            };
            if quote != '"' && quote != '\'' {
                return Err("unquoted attribute value".to_string());
            }
            let mut value = String::new();
            loop {
                let Some((_, ch)) = reader.next_char() else {
                    return Err("unterminated attribute value".to_string());
                };
                if ch == quote {
                    break;
                }
                value.push(ch);
            }
            attrs.push((name, decode_entities(&value)));
        } else {
            attrs.push((name, String::new()));
        }
    }
}

/// The ruby value kinds (the writer's encodings inverted).
#[derive(Clone, Copy, PartialEq)]
enum ValueKind {
    Boolean,
    Logical,
    Enumeration,
    Binary,
    Number,
    List,
    String,
}

const BUILTIN_KINDS: &[(&str, ValueKind)] = &[
    ("binary", ValueKind::Binary),
    ("boolean", ValueKind::Boolean),
    ("logical", ValueKind::Logical),
    ("integer", ValueKind::Number),
    ("real", ValueKind::Number),
    ("number", ValueKind::Number),
    ("string", ValueKind::String),
];

fn perr<T>(message: impl Into<String>) -> Result<T, String> {
    Err(message.into())
}

/// A declaration's declared type kind (the ruby value_kind +
/// resolve_defined_type over the schema wire).
fn value_kind(schema: &Value, declaration: Option<&Value>) -> Result<ValueKind, String> {
    let type_node = declaration.and_then(|d| d.get("type"));
    let Some(type_node) = type_node else {
        return Ok(ValueKind::String);
    };
    let class = type_node
        .get("_class")
        .and_then(Value::as_str)
        .unwrap_or("");
    let kind = match class {
        "Expressir::Model::DataTypes::Boolean" => ValueKind::Boolean,
        "Expressir::Model::DataTypes::Logical" => ValueKind::Logical,
        "Expressir::Model::DataTypes::Binary" => ValueKind::Binary,
        "Expressir::Model::DataTypes::Enumeration" => ValueKind::Enumeration,
        "Expressir::Model::DataTypes::Integer"
        | "Expressir::Model::DataTypes::Real"
        | "Expressir::Model::DataTypes::Number" => ValueKind::Number,
        _ => {
            if type_node.get("base_type").is_some_and(|b| !b.is_null()) {
                ValueKind::List
            } else {
                resolve_defined_type(schema, type_node, 0)?.unwrap_or(ValueKind::String)
            }
        }
    };
    Ok(kind)
}

/// A defined TYPE's underlying kind; depth-guarded against cycles.
fn resolve_defined_type(
    schema: &Value,
    type_node: &Value,
    depth: usize,
) -> Result<Option<ValueKind>, String> {
    if depth > 32 {
        return Ok(None);
    }
    let class = type_node
        .get("_class")
        .and_then(Value::as_str)
        .unwrap_or("");
    match class {
        "Expressir::Model::DataTypes::Enumeration" => Ok(Some(ValueKind::Enumeration)),
        "Expressir::Model::References::SimpleReference" => {
            let id = base_id_of(type_node).unwrap_or_default();
            let downcased = id.to_lowercase();
            if let Some((_, kind)) = BUILTIN_KINDS.iter().find(|(name, _)| *name == downcased) {
                return Ok(Some(*kind));
            }
            let decl = schema
                .get("types")
                .and_then(Value::as_array)
                .and_then(|types| {
                    types.iter().find(|t| {
                        t.get("id")
                            .and_then(Value::as_str)
                            .is_some_and(|tid| tid.to_lowercase() == downcased)
                    })
                });
            match decl.and_then(|d| d.get("underlying_type")) {
                Some(underlying) if !underlying.is_null() => {
                    resolve_defined_type(schema, underlying, depth + 1)
                }
                _ => Ok(None),
            }
        }
        _ => Ok(None),
    }
}

fn base_id_of(type_node: &Value) -> Option<&str> {
    if let Some(base_path) = type_node.get("base_path").and_then(Value::as_str) {
        return base_path.split('.').next_back();
    }
    type_node.get("id").and_then(Value::as_str)
}

/// Direct or inherited explicit Attribute declaration by name.
fn find_attribute_decl<'a>(
    schema_index: &SchemaIndex,
    schema: &'a Value,
    entity_id: &str,
    name: &str,
) -> Option<&'a Value> {
    let key = name.to_lowercase();
    let mut chain: Vec<String> = schema_index
        .closure(entity_id)
        .iter()
        .map(|info| info.id.clone())
        .collect();
    chain.push(entity_id.to_string());
    for id in chain {
        let Some(entity) = schema
            .get("entities")
            .and_then(Value::as_array)
            .and_then(|entities| {
                entities.iter().find(|e| {
                    e.get("id")
                        .and_then(Value::as_str)
                        .is_some_and(|eid| eid.to_lowercase() == id.to_lowercase())
                })
            })
        else {
            continue;
        };
        let found = entity
            .get("attributes")
            .and_then(Value::as_array)
            .and_then(|attrs| {
                attrs.iter().find(|attr| {
                    attr.get("_class").and_then(Value::as_str)
                        == Some("Expressir::Model::Declarations::Attribute")
                        && attr
                            .get("id")
                            .and_then(Value::as_str)
                            .is_some_and(|aid| aid.to_lowercase() == key)
                })
            });
        if found.is_some() {
            return found;
        }
    }
    None
}

fn numeric_text(text: &str) -> Value {
    if text.chars().all(|c| c.is_ascii_digit() || c == '-') && text.matches('-').count() <= 1 {
        if let Ok(value) = text.parse::<i64>() {
            return serde_json::json!({ "number": value.to_string() });
        }
    }
    serde_json::json!({ "number": text })
}

/// The ruby logical_item: the P21 rendering of a logical face.
fn logical_item(text: &str, allowed: &[&str]) -> Result<String, String> {
    let item = match text {
        "true" => "T",
        "false" => "F",
        "unknown" => "UNKNOWN",
        _ => "",
    };
    if allowed.contains(&item) {
        Ok(item.to_string())
    } else {
        perr(format!("invalid logical value '{text}'"))
    }
}

/// The ruby split_list: whitespace LIST text into encoded components.
fn split_list(schema: &Value, declaration: Option<&Value>, text: &str) -> Result<Value, String> {
    if text.is_empty() {
        return Ok(serde_json::json!({ "list": [] }));
    }
    let base = declaration
        .and_then(|d| d.get("type"))
        .and_then(|t| t.get("base_type"));
    let kind = match base {
        Some(base) if !base.is_null() => {
            resolve_defined_type(schema, base, 0)?.unwrap_or(ValueKind::String)
        }
        _ => ValueKind::String,
    };
    let items: Vec<Value> = text
        .split_whitespace()
        .map(|item| match kind {
            ValueKind::Enumeration => serde_json::json!({ "enum": item }),
            ValueKind::Number => numeric_text(item),
            _ => serde_json::json!(item),
        })
        .collect();
    Ok(serde_json::json!({ "list": items }))
}

/// The inverse of the writer's value encodings (the ruby
/// attribute_value) — gate param encoding out.
fn attribute_value(
    schema: &Value,
    schema_index: &SchemaIndex,
    entity_id: &str,
    slot_name: &str,
    accessor: &XNode,
) -> Result<Value, String> {
    let declaration = find_attribute_decl(schema_index, schema, entity_id, slot_name);
    let components: Vec<&XNode> = accessor.elements().collect();
    if !components.is_empty() {
        let mut refs: Vec<Value> = Vec::new();
        for component in components {
            let Some(r) = component.attr("ref") else {
                return perr(format!("component '{}' has no ref", component.name));
            };
            refs.push(serde_json::json!(format!(
                "#{}",
                r.trim_start_matches("id")
            )));
        }
        return Ok(serde_json::json!({ "list": refs }));
    }
    if let Some(r) = accessor.attr("ref") {
        return Ok(serde_json::json!(format!(
            "#{}",
            r.trim_start_matches("id")
        )));
    }
    let text = accessor.text();
    match value_kind(schema, declaration)? {
        ValueKind::Boolean => Ok(serde_json::json!({
            "enum": logical_item(text, &["T", "F"])?
        })),
        ValueKind::Logical => Ok(serde_json::json!({
            "enum": logical_item(text, &["T", "F", "UNKNOWN"])?
        })),
        ValueKind::Enumeration => Ok(serde_json::json!({ "enum": text })),
        ValueKind::Binary => Ok(serde_json::json!({ "binary": text })),
        ValueKind::Number => Ok(numeric_text(text)),
        ValueKind::List => split_list(schema, declaration, text),
        ValueKind::String => Ok(serde_json::json!(text)),
    }
}

/// The ruby Uos.parse: a uos document over a schema wire in, the
/// gate population encoding out.
pub fn uos_parse_json(xml: &str, schema_json: &str) -> Result<String, String> {
    let schema: Value =
        serde_json::from_str(schema_json).map_err(|e| format!("schema json: {e}"))?;
    let population = uos_parse(xml, &schema)?;
    serde_json::to_string(&population).map_err(|e| format!("population json: {e}"))
}

fn uos_parse(xml: &str, schema: &Value) -> Result<Value, String> {
    let doc = parse_xml(xml)?;
    if doc.name != "uos" {
        return perr(format!("not a uos document (root '{}')", doc.name));
    }
    let schema_index = SchemaIndex::new(schema);
    let schema_name = schema.get("id").and_then(Value::as_str).unwrap_or("");
    let mut instances: Vec<Value> = Vec::new();
    for node in doc.elements() {
        let pid = instance_pid(node)?;
        let record = if node.name == "complexEntity" {
            let mut parts: Vec<Value> = Vec::new();
            for role in node.elements() {
                parts.push(role_record(schema, &schema_index, schema_name, role)?);
            }
            serde_json::json!({ "records": parts })
        } else {
            serde_json::json!({ "records": [role_record(schema, &schema_index, schema_name, node)?] })
        };
        instances.push(serde_json::json!({ "id": pid, "records": record["records"] }));
    }
    Ok(serde_json::json!({ "instances": instances }))
}

fn instance_pid(node: &XNode) -> Result<u64, String> {
    let Some(id) = node.attr("id") else {
        return perr(format!("instance element '{}' has no id", node.name));
    };
    let digits = id.trim_start_matches("id");
    digits
        .parse::<u64>()
        .map_err(|_| format!("instance element '{}' has no id", node.name))
}

fn role_record(
    schema: &Value,
    schema_index: &SchemaIndex,
    schema_name: &str,
    node: &XNode,
) -> Result<Value, String> {
    let Some(entity_id) = schema_index
        .entities
        .keys()
        .find(|id| id.as_str() == node.name.to_lowercase())
        .cloned()
    else {
        return perr(format!(
            "element '{}' does not match any entity of schema '{}'",
            node.name, schema_name
        ));
    };
    let entity = schema
        .get("entities")
        .and_then(Value::as_array)
        .and_then(|entities| {
            entities.iter().find(|e| {
                e.get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id.to_lowercase() == entity_id)
            })
        })
        .cloned()
        .unwrap_or(Value::Null);
    let keyword = entity
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_uppercase();
    let slots = schema_index.slots(&entity_id);
    let mut mapped: std::collections::BTreeMap<usize, Value> = std::collections::BTreeMap::new();
    for accessor in node.elements() {
        let slot_name = accessor.name.to_lowercase();
        let Some(index) = slots.iter().position(|slot| *slot == slot_name) else {
            return perr(format!(
                "entity '{}' has no attribute '{}'",
                keyword, accessor.name
            ));
        };
        mapped.insert(
            index,
            attribute_value(schema, schema_index, &entity_id, &slot_name, accessor)?,
        );
    }
    let mut params: Vec<Value> = Vec::with_capacity(slots.len());
    for i in 0..slots.len() {
        params.push(mapped.remove(&i).unwrap_or(serde_json::json!("unset")));
    }
    Ok(serde_json::json!({ "keyword": keyword, "params": params }))
}

/// The ruby Document.parse: an iso_10303_28 envelope in, the express
/// elements + parsed schemas + per-uos populations out.
pub fn document_parse_json(xml: &str) -> Result<String, String> {
    let doc = parse_xml(xml)?;
    if doc.name != "iso_10303_28" {
        return perr(format!(
            "not an iso_10303_28 document (root '{}')",
            doc.name
        ));
    }
    let mut express: Vec<Value> = Vec::new();
    let mut schemas: serde_json::Map<String, Value> = serde_json::Map::new();
    for node in doc.elements() {
        if node.name != "express" {
            continue;
        }
        let element = serde_json::json!({
            "id": node.attr("id").unwrap_or_default(),
            "schema_name": node.attr("schema_name").unwrap_or_default(),
            "source": node.text(),
        });
        let source = node.text();
        let tree = crate::ParsedTree::parse(source)
            .map_err(|e| format!("express source parse failed: {e:?}"))?;
        let wire = tree
            .to_model_json("p28.express")
            .map_err(|e| format!("express wire failed: {e:?}"))?;
        let schema = wire
            .get("schemas")
            .and_then(Value::as_array)
            .and_then(|s| s.first())
            .cloned()
            .unwrap_or(Value::Null);
        schemas.insert(node.attr("id").unwrap_or_default().to_string(), schema);
        express.push(element);
    }
    let mut units: serde_json::Map<String, Value> = serde_json::Map::new();
    for node in doc.elements() {
        if node.name != "uos" {
            continue;
        }
        let id = node.attr("schema").unwrap_or_default().to_string();
        let Some(schema) = schemas.get(&id) else {
            return perr(format!("uos references unknown express id '{id}'"));
        };
        let population = uos_parse(&serialize_node(node), schema)?;
        units.insert(id, population);
    }
    let out = serde_json::json!({
        "express": express,
        "units": Value::Object(units),
    });
    serde_json::to_string(&out).map_err(|e| format!("result json: {e}"))
}

/// Re-serialize a parsed uos element back to XML text (the ruby
/// node.to_xml handoff into Uos.parse).
fn serialize_node(node: &XNode) -> String {
    fn write(node: &XNode, out: &mut String) {
        out.push('<');
        out.push_str(&node.name);
        for (key, value) in &node.attrs {
            out.push(' ');
            out.push_str(key);
            out.push_str("=\"");
            for ch in value.chars() {
                match ch {
                    '&' => out.push_str("&amp;"),
                    '<' => out.push_str("&lt;"),
                    '>' => out.push_str("&gt;"),
                    '"' => out.push_str("&quot;"),
                    _ => out.push(ch),
                }
            }
            out.push('"');
        }
        if node.children.is_empty() && node.text.is_empty() {
            out.push_str("/>");
            return;
        }
        out.push('>');
        out.push_str(&node.text);
        for child in &node.children {
            write(child, out);
        }
        out.push_str(&format!("</{}>", node.name));
    }
    let mut out = String::new();
    write(node, &mut out);
    out
}
