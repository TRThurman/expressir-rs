//! The EXPRESS evaluator (expressir#639, slice 1): ISO 10303-11 ch.12
//! expression evaluation over the model-hash wire — a port of the ruby
//! `Expressir::Eval::Expression` + `Values` (the pure half; instance
//! access raises exactly like the ruby slice-1 lane). The parity gate
//! replays the ruby engine's own results over fixture expressions.
//!
//! Values are the tagged enum below; the gate JSON encoding is
//! documented in `value_to_json`.

use serde_json::{json, Value};

/// The evaluator value model (ISO 10303-11 ch.8).
#[derive(Debug, Clone, PartialEq)]
pub enum EvalValue {
    Indeterminate,
    Logical(Logical),
    Enumeration(String),
    Binary(String),
    Str(String),
    Int(i64),
    Float(f64),
    Aggregate {
        kind: AggKind,
        elements: Vec<EvalValue>,
    },
    /// A Part 21 instance: the record plus its id (the ruby
    /// Values::Instance). Carried through the population lanes.
    Instance {
        id: u64,
        record: crate::population::Instance,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Logical {
    True,
    False,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggKind {
    List,
    Set,
    Bag,
}

/// Wire node classes.
pub(crate) fn class_of(node: &Value) -> &str {
    node.get("_class").and_then(Value::as_str).unwrap_or("")
}

/// Errors mirror the ruby `Expressir::Eval::Error` messages.
pub struct EvalError(pub String);

fn err<T>(message: impl Into<String>) -> Result<T, EvalError> {
    Err(EvalError(message.into()))
}

/// The schema context the interpreter lanes carry: the schema's
/// functions (for call override) and types (for declared-type
/// enumeration literals like `color.red`). The pure expression lane
/// runs with no context — exactly the ruby Environment(schema: nil).
#[derive(Clone, Default)]
pub struct Ctx {
    functions: Vec<Value>,
    types: Vec<Value>,
    entities: Vec<Value>,
    schema_id: String,
    population: Option<crate::population::FullPopulation>,
}

impl Ctx {
    pub fn from_schema(schema: &Value) -> Ctx {
        Ctx {
            functions: schema
                .get("functions")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            types: schema
                .get("types")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            entities: schema
                .get("entities")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            schema_id: schema
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            population: None,
        }
    }

    pub fn with_population(mut self, population: crate::population::FullPopulation) -> Ctx {
        self.population = Some(population);
        self
    }

    pub(crate) fn schema_id(&self) -> &str {
        &self.schema_id
    }

    pub(crate) fn population(&self) -> Option<&crate::population::FullPopulation> {
        self.population.as_ref()
    }

    pub(crate) fn entity_for_keyword(&self, keyword: &str) -> Option<&Value> {
        let key = keyword.to_lowercase();
        self.entities.iter().find(|e| {
            e.get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| id.to_lowercase() == key)
        })
    }

    pub(crate) fn instance_for(&self, id: u64) -> Option<crate::population::Instance> {
        let population = self.population.as_ref()?;
        population
            .instances
            .iter()
            .find(|(instance_id, _)| *instance_id == id)
            .map(|(_, record)| record.clone())
    }

    /// The ruby Environment#slots_for: the supertype closure's
    /// explicit attributes (nearest first) then the entity's own,
    /// indexed in that order. Later duplicate ids overwrite the slot
    /// but keep the first position (ruby Hash semantics).
    pub(crate) fn slots_for(&self, entity: &Value) -> Vec<(String, usize)> {
        let mut slots: Vec<(String, usize)> = Vec::new();
        let mut chain: Vec<&Value> = supertype_closure(self, entity);
        chain.push(entity);
        let mut index = 0usize;
        for candidate in chain {
            for attribute in explicit_attributes(candidate) {
                let name = attribute
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_lowercase();
                if name.is_empty() {
                    index += 1;
                    continue;
                }
                if let Some(slot) = slots.iter_mut().find(|(n, _)| *n == name) {
                    slot.1 = index;
                } else {
                    slots.push((name, index));
                }
                index += 1;
            }
        }
        slots
    }

    pub fn find_function(&self, name: &str) -> Option<&Value> {
        let key = name.to_lowercase();
        self.functions.iter().find(|f| {
            f.get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| id.to_lowercase() == key)
        })
    }

    pub fn find_enum_type(&self, name: &str) -> Option<&Value> {
        let key = name.to_lowercase();
        self.types.iter().find(|t| {
            t.get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| id.to_lowercase() == key)
                && t.get("underlying_type")
                    .is_some_and(|u| class_of(u).ends_with("::Enumeration"))
        })
    }
}

/// The scope chain: the harness pre-seeds variables; QUERY pushes a
/// frame (the ruby Environment.push).
#[derive(Default)]
pub struct Env {
    frames: Vec<Vec<(String, EvalValue)>>,
    ctx: Option<Ctx>,
}

impl Env {
    pub fn with_variables(variables: Vec<(String, EvalValue)>) -> Self {
        Env {
            frames: vec![variables],
            ctx: None,
        }
    }

    pub fn with_variables_ctx(variables: Vec<(String, EvalValue)>, ctx: Ctx) -> Self {
        Env {
            frames: vec![variables],
            ctx: Some(ctx),
        }
    }

    pub(crate) fn ctx(&self) -> Option<&Ctx> {
        self.ctx.as_ref()
    }

    pub(crate) fn define(&mut self, name: &str, value: EvalValue) {
        let key = name.to_lowercase();
        match self.frames.last_mut() {
            Some(frame) => {
                if let Some(slot) = frame.iter_mut().find(|(n, _)| *n == key) {
                    slot.1 = value;
                } else {
                    frame.push((key, value));
                }
            }
            None => self.frames.push(vec![(key, value)]),
        }
    }

    pub(crate) fn lookup(&self, name: &str) -> Option<&EvalValue> {
        let key = name.to_lowercase();
        self.frames
            .iter()
            .rev()
            .find_map(|frame| frame.iter().find(|(n, _)| *n == key).map(|(_, v)| v))
    }

    fn pushed(&self, name: &str, value: EvalValue) -> Env {
        let mut frames = self.frames.clone();
        frames.push(vec![(name.to_lowercase(), value)]);
        Env {
            frames,
            ctx: self.ctx.clone(),
        }
    }
}

/// The gate JSON encoding of a value.
pub fn value_to_json(value: &EvalValue) -> Value {
    match value {
        EvalValue::Indeterminate => json!({ "indeterminate": true }),
        EvalValue::Logical(Logical::True) => json!({ "logical": "TRUE" }),
        EvalValue::Logical(Logical::False) => json!({ "logical": "FALSE" }),
        EvalValue::Logical(Logical::Unknown) => json!({ "logical": "UNKNOWN" }),
        EvalValue::Enumeration(item) => json!({ "enum": item }),
        EvalValue::Binary(bits) => json!({ "binary": bits }),
        EvalValue::Str(s) => json!(s),
        EvalValue::Int(i) => json!(i),
        EvalValue::Float(f) => json!(f),
        EvalValue::Aggregate { kind, elements } => json!({
            "aggregate": {
                "kind": match kind {
                    AggKind::List => "list",
                    AggKind::Set => "set",
                    AggKind::Bag => "bag",
                },
                "elements": elements.iter().map(value_to_json).collect::<Vec<_>>(),
            }
        }),
        // The gate encoding of an instance is its #id tag, exactly the
        // ruby Values::Instance#to_s.
        EvalValue::Instance { id, .. } => json!(format!("#{id}")),
    }
}

/// The delegation entry: a wire expression node + a JSON variable map
/// (`name → value` in the gate encoding) in, the result value out.
pub fn evaluate_json(expression_json: &str, variables_json: &str) -> Result<String, String> {
    let expression: Value =
        serde_json::from_str(expression_json).map_err(|e| format!("expression json: {e}"))?;
    let variables: serde_json::Map<String, Value> =
        serde_json::from_str(variables_json).map_err(|e| format!("variables json: {e}"))?;
    let seed = variables
        .into_iter()
        .map(|(name, wire)| value_from_json(&wire).map(|v| (name, v)))
        .collect::<Result<Vec<_>, _>>()?;
    let value = evaluate(&expression, &mut Env::with_variables(seed)).map_err(|e| e.0)?;
    serde_json::to_string(&value_to_json(&value)).map_err(|e| format!("result json: {e}"))
}

pub fn value_from_json(wire: &Value) -> Result<EvalValue, String> {
    if let Some(object) = wire.as_object() {
        if object.get("indeterminate").and_then(Value::as_bool) == Some(true) {
            return Ok(EvalValue::Indeterminate);
        }
        if let Some(logical) = object.get("logical").and_then(Value::as_str) {
            return Ok(EvalValue::Logical(match logical {
                "TRUE" => Logical::True,
                "FALSE" => Logical::False,
                _ => Logical::Unknown,
            }));
        }
        if let Some(item) = object.get("enum").and_then(Value::as_str) {
            return Ok(EvalValue::Enumeration(item.to_string()));
        }
        if let Some(bits) = object.get("binary").and_then(Value::as_str) {
            return Ok(EvalValue::Binary(bits.to_string()));
        }
        if let Some(aggregate) = object.get("aggregate") {
            let kind = match aggregate["kind"].as_str() {
                Some("set") => AggKind::Set,
                Some("bag") => AggKind::Bag,
                _ => AggKind::List,
            };
            let elements = aggregate["elements"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .map(value_from_json)
                        .collect::<Result<Vec<_>, _>>()
                })
                .unwrap_or(Ok(Vec::new()))?;
            return Ok(EvalValue::Aggregate { kind, elements });
        }
    }
    if let Some(number) = wire.as_i64() {
        return Ok(EvalValue::Int(number));
    }
    if let Some(number) = wire.as_f64() {
        return Ok(EvalValue::Float(number));
    }
    if let Some(text) = wire.as_str() {
        return Ok(EvalValue::Str(text.to_string()));
    }
    Err("unsupported variable value".to_string())
}

// ---- literals + dispatch ----

pub fn evaluate(node: &Value, env: &mut Env) -> Result<EvalValue, EvalError> {
    match class_of(node).rsplit("::").next().unwrap_or("") {
        "Integer" => Ok(EvalValue::Int(
            node.get("value")
                .and_then(Value::as_str)
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| EvalError("bad integer literal".into()))?,
        )),
        "Real" => Ok(EvalValue::Float(
            node.get("value")
                .and_then(Value::as_str)
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| EvalError("bad real literal".into()))?,
        )),
        "String" => Ok(EvalValue::Str(
            node.get("value")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        )),
        "Binary" => Ok(EvalValue::Binary(
            node.get("value")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        )),
        "Logical" => Ok(
            if node.get("value").and_then(Value::as_str) == Some("TRUE") {
                EvalValue::Logical(Logical::True)
            } else {
                EvalValue::Logical(Logical::False)
            },
        ),
        "BinaryExpression" => evaluate_binary(node, env),
        "UnaryExpression" => evaluate_unary(node, env),
        "Interval" => evaluate_interval(node, env),
        "AggregateInitializer" => evaluate_aggregate_initializer(node, env),
        "QueryExpression" => evaluate_query(node, env),
        "FunctionCall" => evaluate_call(node, env),
        "SimpleReference" => evaluate_reference(node, env),
        "AttributeReference" => evaluate_attribute_reference(node, env),
        "IndexReference" => evaluate_index(node, env),
        "GroupReference" => evaluate_group_reference(node, env),
        _other => err(format!("unsupported expression {}", class_of(node))),
    }
}

fn evaluate_reference(node: &Value, env: &mut Env) -> Result<EvalValue, EvalError> {
    let name = node.get("id").and_then(Value::as_str).unwrap_or("");
    match env.lookup(name) {
        Some(value) => Ok(value.clone()),
        None => err(format!("unbound identifier '{name}'")),
    }
}

/// `color.red` (enumeration members via a declared type — schema
/// context arrives with the population slice) or `instance.attr`.
fn evaluate_attribute_reference(node: &Value, env: &mut Env) -> Result<EvalValue, EvalError> {
    let base = node.get("ref").unwrap_or(&Value::Null);
    if class_of(base).ends_with("::SimpleReference") {
        if let Some(ctx) = env.ctx() {
            if let Some(_declaration) =
                ctx.find_enum_type(base.get("id").and_then(Value::as_str).unwrap_or(""))
            {
                let item = node
                    .get("attribute")
                    .and_then(|a| a.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                return Ok(EvalValue::Enumeration(item));
            }
        }
    }
    let base_value = evaluate(base, env)?;
    if let EvalValue::Instance { record, .. } = &base_value {
        let attribute = node
            .get("attribute")
            .and_then(|a| a.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if let Some(ctx) = env.ctx() {
            return instance_attribute(record, attribute, ctx);
        }
    }
    if matches!(base_value, EvalValue::Indeterminate) {
        return Ok(EvalValue::Indeterminate);
    }
    err("instance attribute access requires a population")
}

fn evaluate_group_reference(node: &Value, env: &mut Env) -> Result<EvalValue, EvalError> {
    let base = node.get("ref").unwrap_or(&Value::Null);
    let is_self = class_of(base).ends_with("::SimpleReference")
        && base
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id.eq_ignore_ascii_case("self"));
    if is_self {
        if let Some(value) = env.lookup("self") {
            return Ok(value.clone());
        }
        return err("SELF requires an instance context (step 4)");
    }
    err("instance access requires a population")
}

fn evaluate_aggregate_initializer(node: &Value, env: &mut Env) -> Result<EvalValue, EvalError> {
    let mut elements = Vec::new();
    if let Some(items) = node.get("items").and_then(Value::as_array) {
        for item in items {
            // Items may be wrapped expression rows or bare expressions.
            let expression = item.get("expression").unwrap_or(item);
            elements.push(evaluate(expression, env)?);
        }
    }
    Ok(aggregate(AggKind::List, elements))
}

fn evaluate_index(node: &Value, env: &mut Env) -> Result<EvalValue, EvalError> {
    let base = evaluate(node.get("ref").unwrap_or(&Value::Null), env)?;
    let index1 = evaluate(node.get("index1").unwrap_or(&Value::Null), env)?;
    let index2 = match node.get("index2") {
        Some(i) if !i.is_null() => Some(evaluate(i, env)?),
        _ => None,
    };
    match &base {
        EvalValue::Str(text) => {
            let chars: Vec<char> = text.chars().collect();
            Ok(EvalValue::Str(string_slice_chars(
                &chars,
                &index1,
                index2.as_ref(),
            )?))
        }
        EvalValue::Binary(bits) => {
            let chars: Vec<char> = bits.chars().collect();
            Ok(EvalValue::Binary(string_slice_chars(
                &chars,
                &index1,
                index2.as_ref(),
            )?))
        }
        EvalValue::Aggregate { elements, .. } => {
            if index2.is_some() {
                return err("aggregate indexing selects one element (12.6.1)");
            }
            let EvalValue::Int(index) = to_number(&index1)? else {
                return err("expected a numeric operand, got index");
            };
            elements
                .get(index as usize - 1)
                .cloned()
                .ok_or_else(|| EvalError("index out of range".into()))
        }
        other => err(format!("cannot index {}", describe(other))),
    }
}

fn string_slice_chars(
    chars: &[char],
    index1: &EvalValue,
    index2: Option<&EvalValue>,
) -> Result<String, EvalError> {
    let EvalValue::Int(i1) = to_number(index1)? else {
        return err("expected a numeric operand, got index");
    };
    let start = i1 as usize - 1;
    match index2 {
        None => Ok(chars.get(start).map(|c| c.to_string()).unwrap_or_default()),
        Some(i2v) => {
            let EvalValue::Int(i2) = to_number(i2v)? else {
                return err("expected a numeric operand, got index");
            };
            if i2 < i1 {
                return Ok(String::new());
            }
            let stop = (i2 as usize).min(chars.len());
            Ok(chars[start.min(chars.len())..stop.min(chars.len())]
                .iter()
                .collect())
        }
    }
}

fn evaluate_query(node: &Value, env: &mut Env) -> Result<EvalValue, EvalError> {
    let source = evaluate(node.get("aggregate_source").unwrap_or(&Value::Null), env)?;
    let EvalValue::Aggregate { kind, elements } = &source else {
        return err("QUERY source shall be an aggregate (12.6.7)");
    };
    let variable = node
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let expression = node.get("expression").unwrap_or(&Value::Null);
    let kept: Vec<EvalValue> = elements
        .clone()
        .into_iter()
        .filter(|element| {
            let mut inner = env.pushed(&variable, element.clone());
            matches!(evaluate(expression, &mut inner), Ok(v) if is_logical_true(&v))
        })
        .collect();
    Ok(aggregate(*kind, kept))
}

fn evaluate_call(node: &Value, env: &mut Env) -> Result<EvalValue, EvalError> {
    let callee = node.get("function").unwrap_or(&Value::Null);
    let name = callee.get("id").and_then(Value::as_str).unwrap_or("");
    if name.is_empty() {
        return err(format!("unsupported call target {}", class_of(callee)));
    }
    let mut arguments = Vec::new();
    if let Some(params) = node.get("parameters").and_then(Value::as_array) {
        for param in params {
            arguments.push(evaluate(param, env)?);
        }
    }
    let ctx = env.ctx();
    if let Some(ctx) = ctx {
        if let Some(function) = ctx.find_function(name) {
            // A schema function gets a fresh scope (the ruby
            // Environment.new inside execute_function) — a bare RETURN
            // surfaces as Indeterminate here; the interpreter gate owns
            // the nil-encoding contract.
            return crate::eval_interpreter::call_function(function, arguments, ctx)
                .map(|value| value.unwrap_or(EvalValue::Indeterminate));
        }
    }
    crate::eval_builtins::call_builtin(name, &arguments, ctx)
}

fn evaluate_unary(node: &Value, env: &mut Env) -> Result<EvalValue, EvalError> {
    let operand = evaluate(node.get("operand").unwrap_or(&Value::Null), env)?;
    if matches!(operand, EvalValue::Indeterminate) {
        return Ok(EvalValue::Indeterminate);
    }
    match node.get("operator").and_then(Value::as_str) {
        Some("PLUS") => to_number(&operand),
        Some("MINUS") => to_number(&operand).map(|n| match n {
            EvalValue::Int(i) => EvalValue::Int(-i),
            EvalValue::Float(f) => EvalValue::Float(-f),
            other => other,
        }),
        Some("NOT") => {
            let logical = as_logical(&operand)?;
            Ok(match logical {
                Logical::True => EvalValue::Logical(Logical::False),
                Logical::False => EvalValue::Logical(Logical::True),
                Logical::Unknown => EvalValue::Logical(Logical::Unknown),
            })
        }
        other => err(format!("unsupported unary operator {other:?}")),
    }
}

fn evaluate_interval(node: &Value, env: &mut Env) -> Result<EvalValue, EvalError> {
    let low = evaluate(node.get("low").unwrap_or(&Value::Null), env)?;
    let item = evaluate(node.get("item").unwrap_or(&Value::Null), env)?;
    let high = evaluate(node.get("high").unwrap_or(&Value::Null), env)?;
    let operator1 = node.get("operator1").and_then(Value::as_str).unwrap_or("");
    let operator2 = node.get("operator2").and_then(Value::as_str).unwrap_or("");
    let left = relational(&low, operator1, &item)?;
    let right = relational(&item, operator2, &high)?;
    evaluate_logical("AND", left, right)
}

fn evaluate_binary(node: &Value, env: &mut Env) -> Result<EvalValue, EvalError> {
    let operator = node.get("operator").and_then(Value::as_str).unwrap_or("");
    let left = evaluate(node.get("operand1").unwrap_or(&Value::Null), env)?;
    let right = evaluate(node.get("operand2").unwrap_or(&Value::Null), env)?;
    if matches!(operator, "AND" | "OR" | "XOR") {
        return evaluate_logical(operator, left, right);
    }
    if matches!(left, EvalValue::Indeterminate) || matches!(right, EvalValue::Indeterminate) {
        return Ok(EvalValue::Logical(Logical::Unknown));
    }
    match operator {
        "ADDITION" => evaluate_addition(left, right),
        "SUBTRACTION" => evaluate_difference(left, right),
        "MULTIPLICATION" => evaluate_intersection(left, right),
        "REAL_DIVISION" => {
            let result = to_number(&left)?.to_float()? / to_number(&right)?.to_float()?;
            Ok(EvalValue::Float(result))
        }
        "INTEGER_DIVISION" | "MODULO" => evaluate_div_mod(operator, left, right),
        "EXPONENTIATION" => {
            let base = to_number(&left)?;
            let exponent = to_number(&right)?;
            match (base, exponent) {
                (EvalValue::Int(b), EvalValue::Int(e)) if e >= 0 => {
                    Ok(EvalValue::Int(b.pow(e as u32)))
                }
                (b, e) => Ok(EvalValue::Float(b.to_float()?.powf(e.to_float()?))),
            }
        }
        "COMBINE" => err("complex entity construction requires instances (step 3)"),
        other => evaluate_relational(other, left, right),
    }
}

fn evaluate_div_mod(
    operator: &str,
    left: EvalValue,
    right: EvalValue,
) -> Result<EvalValue, EvalError> {
    let first = truncate(left)?;
    let second = truncate(right)?;
    if second == 0 {
        return err("division by zero");
    }
    if operator == "INTEGER_DIVISION" {
        Ok(EvalValue::Int(first.div_euclid(second)))
    } else {
        // Ruby %: the result carries the divisor's sign (floored).
        Ok(EvalValue::Int(first.rem_euclid(second)))
    }
}

// ---- arithmetic (12.1) ----

fn arithmetic(left: &EvalValue, right: &EvalValue, op: char) -> Result<EvalValue, EvalError> {
    let a = to_number(left)?;
    let b = to_number(right)?;
    match (a, b, op) {
        (EvalValue::Int(x), EvalValue::Int(y), '+') => Ok(EvalValue::Int(x + y)),
        (EvalValue::Int(x), EvalValue::Int(y), '-') => Ok(EvalValue::Int(x - y)),
        (EvalValue::Int(x), EvalValue::Int(y), '*') => Ok(EvalValue::Int(x * y)),
        (x, y, '+') => Ok(EvalValue::Float(x.to_float()? + y.to_float()?)),
        (x, y, '-') => Ok(EvalValue::Float(x.to_float()? - y.to_float()?)),
        (x, y, '*') => Ok(EvalValue::Float(x.to_float()? * y.to_float()?)),
        _ => err("bad arithmetic operands"),
    }
}

fn truncate(value: EvalValue) -> Result<i64, EvalError> {
    match to_number(&value)? {
        EvalValue::Int(i) => Ok(i),
        EvalValue::Float(f) => Ok(f.trunc() as i64),
        other => err(format!(
            "expected a numeric operand, got {}",
            describe(&other)
        )),
    }
}

fn to_number(value: &EvalValue) -> Result<EvalValue, EvalError> {
    match value {
        EvalValue::Int(_) | EvalValue::Float(_) => Ok(value.clone()),
        other => err(format!(
            "expected a numeric operand, got {}",
            describe(other)
        )),
    }
}

fn evaluate_addition(left: EvalValue, right: EvalValue) -> Result<EvalValue, EvalError> {
    match (&left, &right) {
        (EvalValue::Int(_), EvalValue::Int(_)) => arithmetic(&left, &right, '+'),
        (EvalValue::Float(_), _) | (_, EvalValue::Float(_))
            if to_number(&left).is_ok() && to_number(&right).is_ok() =>
        {
            arithmetic(&left, &right, '+')
        }
        (EvalValue::Str(a), EvalValue::Str(b)) => Ok(EvalValue::Str(format!("{a}{b}"))),
        (EvalValue::Binary(a), EvalValue::Binary(b)) => {
            Ok(EvalValue::Binary(format!("{a}{b}")))
        }
        (a, b) if matches!(a, EvalValue::Aggregate { .. }) || matches!(b, EvalValue::Aggregate { .. }) => {
            union(&left, &right)
        }
        _ => err(
            "addition requires numeric, string, binary or aggregate operands (12.1, 12.3.2, 12.5.2, 12.6)",
        ),
    }
}

fn evaluate_difference(left: EvalValue, right: EvalValue) -> Result<EvalValue, EvalError> {
    match (&left, &right) {
        (a, b) if is_numeric_pair(a, b) => arithmetic(&left, &right, '-'),
        (a, b)
            if matches!(a, EvalValue::Aggregate { .. })
                || matches!(b, EvalValue::Aggregate { .. }) =>
        {
            aggregate_difference(&left, &right)
        }
        _ => err("difference requires numeric or aggregate operands (12.1, 12.6)"),
    }
}

fn evaluate_intersection(left: EvalValue, right: EvalValue) -> Result<EvalValue, EvalError> {
    match (&left, &right) {
        (a, b) if is_numeric_pair(a, b) => arithmetic(&left, &right, '*'),
        (a, b)
            if matches!(a, EvalValue::Aggregate { .. })
                && matches!(b, EvalValue::Aggregate { .. }) =>
        {
            aggregate_intersection(&left, &right)
        }
        _ => err("intersection requires numeric or aggregate operands (12.1, 12.6)"),
    }
}

fn is_numeric_pair(left: &EvalValue, right: &EvalValue) -> bool {
    matches!(left, EvalValue::Int(_) | EvalValue::Float(_))
        && matches!(right, EvalValue::Int(_) | EvalValue::Float(_))
}

// ---- logical operators (12.4) ----

fn evaluate_logical(
    operator: &str,
    left: EvalValue,
    right: EvalValue,
) -> Result<EvalValue, EvalError> {
    let left = as_logical(&left)?;
    let right = as_logical(&right)?;
    if left == Logical::Unknown || right == Logical::Unknown {
        return Ok(EvalValue::Logical(Logical::Unknown));
    }
    Ok(EvalValue::Logical(match operator {
        "AND" => {
            if left == Logical::True && right == Logical::True {
                Logical::True
            } else {
                Logical::False
            }
        }
        "OR" => {
            if left == Logical::True || right == Logical::True {
                Logical::True
            } else {
                Logical::False
            }
        }
        _ => {
            if left == right {
                Logical::False
            } else {
                Logical::True
            }
        }
    }))
}

fn as_logical(value: &EvalValue) -> Result<Logical, EvalError> {
    match value {
        EvalValue::Logical(logical) => Ok(*logical),
        other => err(format!(
            "logical operator requires logical operands (12.4), got {}",
            describe(other)
        )),
    }
}

fn is_logical_true(value: &EvalValue) -> bool {
    matches!(value, EvalValue::Logical(Logical::True))
}

// ---- relational (12.2) ----

fn evaluate_relational(
    operator: &str,
    left: EvalValue,
    right: EvalValue,
) -> Result<EvalValue, EvalError> {
    match operator {
        "IN" => evaluate_in(left, right),
        "LIKE" => {
            let hit = like_match(&left, &right)?;
            Ok(EvalValue::Logical(if hit {
                Logical::True
            } else {
                Logical::False
            }))
        }
        "INSTANCE_EQUAL" | "INSTANCE_NOT_EQUAL" => {
            err("instance comparison requires instances (step 3)")
        }
        other => {
            let symbol = match other {
                "EQUAL" | "NOT_EQUAL" => "=",
                "LESS_THAN" => "<",
                "LESS_THAN_OR_EQUAL" => "<=",
                "GREATER_THAN" => ">",
                "GREATER_THAN_OR_EQUAL" => ">=",
                unknown => return err(format!("unsupported relational operator {unknown}")),
            };
            let result = relational(&left, symbol, &right)?;
            if other == "NOT_EQUAL" {
                let negated = match result {
                    EvalValue::Logical(logical) => negate(logical),
                    other => return err(format!("negation of {}", describe(&other))),
                };
                Ok(EvalValue::Logical(negated))
            } else {
                Ok(result)
            }
        }
    }
}

fn relational(left: &EvalValue, operator: &str, right: &EvalValue) -> Result<EvalValue, EvalError> {
    if matches!(left, EvalValue::Indeterminate) || matches!(right, EvalValue::Indeterminate) {
        return Ok(EvalValue::Logical(Logical::Unknown));
    }
    let ordering = order(left, right)?;
    let result = match operator {
        "=" => ordering == 0,
        "<" => ordering < 0,
        "<=" => ordering <= 0,
        ">" => ordering > 0,
        ">=" => ordering >= 0,
        // The interval lane hands raw wire strings here; ruby's symbol
        // case falls through to nil (falsy) — FALSE, not an error.
        _ => false,
    };
    Ok(EvalValue::Logical(if result {
        Logical::True
    } else {
        Logical::False
    }))
}

fn negate(logical: Logical) -> Logical {
    match logical {
        Logical::True => Logical::False,
        Logical::False => Logical::True,
        Logical::Unknown => Logical::Unknown,
    }
}

/// 12.2.1 orderings: numeric, string (code point), binary (bit),
/// logical (false < unknown < true), enumeration (item name).
fn order(left: &EvalValue, right: &EvalValue) -> Result<i32, EvalError> {
    match (left, right) {
        (EvalValue::Int(a), EvalValue::Int(b)) => Ok(cmp_i32(a.cmp(b))),
        (EvalValue::Float(a), EvalValue::Float(b)) => Ok(cmp_i32(
            a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal),
        )),
        (EvalValue::Int(a), EvalValue::Float(b)) => Ok(cmp_i32(
            (*a as f64)
                .partial_cmp(b)
                .unwrap_or(std::cmp::Ordering::Equal),
        )),
        (EvalValue::Float(a), EvalValue::Int(b)) => Ok(cmp_i32(
            a.partial_cmp(&(*b as f64))
                .unwrap_or(std::cmp::Ordering::Equal),
        )),
        (EvalValue::Str(a), EvalValue::Str(b)) => Ok(cmp_i32(a.cmp(b))),
        (EvalValue::Binary(a), EvalValue::Binary(b)) => Ok(cmp_i32(a.cmp(b))),
        (EvalValue::Logical(a), EvalValue::Logical(b)) => {
            Ok(cmp_i32(logical_order(*a).cmp(&logical_order(*b))))
        }
        (EvalValue::Enumeration(a), EvalValue::Enumeration(b)) => Ok(cmp_i32(a.cmp(b))),
        (a, b) => err(format!(
            "cannot compare {} with {}",
            describe(a),
            describe(b)
        )),
    }
}

fn cmp_i32(ordering: std::cmp::Ordering) -> i32 {
    match ordering {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

fn logical_order(logical: Logical) -> u8 {
    match logical {
        Logical::False => 0,
        Logical::Unknown => 1,
        Logical::True => 2,
    }
}

// ---- LIKE (12.2.5, table 11) ----

fn like_match(left: &EvalValue, right: &EvalValue) -> Result<bool, EvalError> {
    match (left, right) {
        (EvalValue::Str(target), EvalValue::Str(pattern)) => {
            let pattern_chars: Vec<char> = pattern.chars().collect();
            Ok(match_sequence(target, &pattern_chars, 0))
        }
        _ => err("like requires string operands (12.2.5)"),
    }
}

fn match_sequence(target: &str, pattern: &[char], offset: usize) -> bool {
    if pattern.is_empty() {
        return offset == target.chars().count();
    }
    let tail = &pattern[1..];
    match pattern[0] {
        '\\' => {
            let Some(escaped) = pattern.get(1).copied() else {
                return false;
            };
            match_single(target, &pattern[2..], offset, |tc| tc == escaped)
        }
        '*' => (offset..=target.chars().count()).any(|split| match_sequence(target, tail, split)),
        '&' => match_sequence(target, tail, target.chars().count()),
        '$' => match_dollar(target, tail, offset),
        '!' => match_negation(target, tail, offset),
        other => match_single(target, tail, offset, |tc| char_matches(other, tc)),
    }
}

fn match_dollar(target: &str, pattern: &[char], offset: usize) -> bool {
    let chars: Vec<char> = target.chars().collect();
    let index = chars[offset.min(chars.len())..]
        .iter()
        .position(|c| *c == ' ')
        .map(|p| p + offset)
        .unwrap_or(chars.len());
    let prefix: String = chars[offset.min(index)..index].iter().collect();
    let remaining: String = chars[index.min(chars.len())..].iter().collect();
    match_sequence(&prefix, pattern, 0) && match_sequence(&remaining, &[], 0)
}

fn match_negation(target: &str, pattern: &[char], offset: usize) -> bool {
    let Some(negated) = pattern.first().copied() else {
        return false;
    };
    let chars: Vec<char> = target.chars().collect();
    if offset >= chars.len() {
        return false;
    }
    let target_char = chars[offset];
    let matches = match negated {
        '@' => target_char.is_alphabetic(),
        '^' => target_char.is_uppercase(),
        '#' => target_char.is_ascii_digit(),
        '?' => true,
        other => target_char == other,
    };
    if matches {
        return false;
    }
    match_sequence(target, &pattern[1..], offset + 1)
}

fn match_single(
    target: &str,
    pattern: &[char],
    offset: usize,
    matches: impl Fn(char) -> bool,
) -> bool {
    let chars: Vec<char> = target.chars().collect();
    if offset >= chars.len() {
        return false;
    }
    matches(chars[offset]) && match_sequence(target, pattern, offset + 1)
}

fn char_matches(pattern_char: char, target_char: char) -> bool {
    match pattern_char {
        '@' => target_char.is_alphabetic(),
        '^' => target_char.is_uppercase(),
        '#' => target_char.is_ascii_digit(),
        '?' => true,
        literal => pattern_char == target_char && literal == target_char,
    }
}

// ---- aggregates (12.6, tables 16-18) ----

fn aggregate(kind: AggKind, elements: Vec<EvalValue>) -> EvalValue {
    EvalValue::Aggregate { kind, elements }
}

fn aggregate_of(kind: AggKind, elements: Vec<EvalValue>) -> EvalValue {
    if kind == AggKind::Set {
        let mut deduped: Vec<EvalValue> = Vec::new();
        for element in elements {
            if !deduped.iter().any(|seen| values_equal(seen, &element)) {
                deduped.push(element);
            }
        }
        return aggregate(kind, deduped);
    }
    aggregate(kind, elements)
}

fn union(left: &EvalValue, right: &EvalValue) -> Result<EvalValue, EvalError> {
    let left_elements: Vec<EvalValue> = match left {
        EvalValue::Aggregate { elements, .. } => elements.clone(),
        other => vec![other.clone()],
    };
    let right_elements: Vec<EvalValue> = match right {
        EvalValue::Aggregate { kind, elements } => {
            let _ = kind;
            elements.clone()
        }
        other => vec![other.clone()],
    };
    let kind_of = |v: &EvalValue| match v {
        EvalValue::Aggregate { kind, .. } => Some(*kind),
        _ => None,
    };
    let mut kind = AggKind::List;
    if kind_of(left) == Some(AggKind::Bag) || kind_of(right) == Some(AggKind::Bag) {
        kind = AggKind::Bag;
    } else if kind_of(left) == Some(AggKind::Set) || kind_of(right) == Some(AggKind::Set) {
        kind = AggKind::Set;
    }
    let mut elements = left_elements;
    elements.extend(right_elements);
    Ok(aggregate_of(kind, elements))
}

fn aggregate_difference(left: &EvalValue, right: &EvalValue) -> Result<EvalValue, EvalError> {
    let kind = match (left, right) {
        (EvalValue::Aggregate { kind, .. }, _) => *kind,
        (_, EvalValue::Aggregate { kind, .. }) => *kind,
        _ => return err("difference requires bag or set operands (12.6, table 18)"),
    };
    if !matches!(kind, AggKind::Bag | AggKind::Set) {
        return err("difference requires bag or set operands (12.6, table 18)");
    }
    let left_elements: Vec<EvalValue> = match left {
        EvalValue::Aggregate { elements, .. } => elements.clone(),
        other => vec![other.clone()],
    };
    let right_elements: Vec<EvalValue> = match right {
        EvalValue::Aggregate { elements, .. } => elements.clone(),
        other => vec![other.clone()],
    };
    let remaining: Vec<EvalValue> = left_elements
        .into_iter()
        .filter(|element| !right_elements.iter().any(|r| values_equal(r, element)))
        .collect();
    Ok(aggregate(kind, remaining))
}

fn aggregate_intersection(left: &EvalValue, right: &EvalValue) -> Result<EvalValue, EvalError> {
    let (
        EvalValue::Aggregate {
            kind: left_kind,
            elements: left_elements,
        },
        EvalValue::Aggregate {
            kind: right_kind,
            elements: right_elements,
        },
    ) = (left, right)
    else {
        return err("intersection requires bag or set operands (12.6, table 16)");
    };
    if !matches!(left_kind, AggKind::Bag | AggKind::Set)
        || !matches!(right_kind, AggKind::Bag | AggKind::Set)
    {
        return err("intersection requires bag or set operands (12.6, table 16)");
    }
    let kind = if *left_kind == AggKind::Bag && *right_kind == AggKind::Bag {
        AggKind::Bag
    } else {
        AggKind::Set
    };
    let kept: Vec<EvalValue> = left_elements
        .iter()
        .filter(|element| right_elements.iter().any(|r| values_equal(r, element)))
        .cloned()
        .collect();
    Ok(aggregate_of(kind, kept))
}

/// The ruby Entity#supertype_closure: BFS over subtype_of references
/// resolved within the schema's entities, visited-set guarded.
pub(crate) fn supertype_closure<'a>(ctx: &'a Ctx, entity: &Value) -> Vec<&'a Value> {
    let mut closure: Vec<&Value> = Vec::new();
    let mut visited: Vec<String> = vec![entity
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase()];
    let mut queue: Vec<String> = entity
        .get("subtype_of")
        .and_then(Value::as_array)
        .map(|refs| {
            refs.iter()
                .filter_map(|r| r.get("id").and_then(Value::as_str))
                .map(|id| id.to_lowercase())
                .collect()
        })
        .unwrap_or_default();
    while let Some(name) = queue.first().cloned() {
        queue.remove(0);
        if name.is_empty() || visited.contains(&name) {
            continue;
        }
        let Some(supertype) = ctx.entity_for_keyword(&name) else {
            continue;
        };
        visited.push(name);
        closure.push(supertype);
        if let Some(refs) = supertype.get("subtype_of").and_then(Value::as_array) {
            for r in refs {
                if let Some(id) = r.get("id").and_then(Value::as_str) {
                    queue.push(id.to_lowercase());
                }
            }
        }
    }
    closure
}

/// The ruby explicit_attributes: EXACT class Attribute — derived,
/// inverse and explicit-with-kind decorations live elsewhere.
fn explicit_attributes(entity: &Value) -> Vec<&Value> {
    entity
        .get("attributes")
        .and_then(Value::as_array)
        .map(|attrs| {
            attrs
                .iter()
                .filter(|a| class_of(a) == "Expressir::Model::Declarations::Attribute")
                .collect()
        })
        .unwrap_or_default()
}

/// The ruby Expression.instance_attribute: resolve the record part
/// that carries the slot, then convert its parameter.
pub(crate) fn instance_attribute(
    instance: &crate::population::Instance,
    name: &str,
    ctx: &Ctx,
) -> Result<EvalValue, EvalError> {
    let record = match instance {
        crate::population::Instance::Simple(record) => record,
        crate::population::Instance::Complex(records) => {
            let key = name.to_lowercase();
            let found = records.iter().find(|record| {
                ctx.entity_for_keyword(&record.keyword)
                    .is_some_and(|entity| {
                        ctx.slots_for(entity).iter().any(|(slot, _)| *slot == key)
                    })
            });
            match found {
                Some(record) => record,
                None => return err(format!("attribute '{name}' is not mapped on the record")),
            }
        }
    };
    let Some(entity) = ctx.entity_for_keyword(&record.keyword) else {
        return err(format!("unknown entity '{}'", record.keyword));
    };
    let key = name.to_lowercase();
    let index = ctx
        .slots_for(entity)
        .into_iter()
        .find(|(slot, _)| *slot == key)
        .map(|(_, index)| index)
        .ok_or_else(|| {
            EvalError(format!(
                "attribute '{name}' is not mapped on '{}'",
                record.keyword
            ))
        })?;
    match record.params.get(index) {
        Some(param) => instance_value(param, ctx),
        None => Ok(EvalValue::Indeterminate),
    }
}

/// The ruby Expression.instance_value: P21 data becomes evaluator
/// values at the instance boundary.
pub(crate) fn instance_value(
    raw: &crate::population::P21Value,
    ctx: &Ctx,
) -> Result<EvalValue, EvalError> {
    use crate::population::P21Value;
    Ok(match raw {
        P21Value::Unset | P21Value::Derived => EvalValue::Indeterminate,
        P21Value::Bool(value) => EvalValue::Logical(if *value {
            Logical::True
        } else {
            Logical::False
        }),
        P21Value::Enum(name) => EvalValue::Enumeration(name.clone()),
        P21Value::Binary(bits) => EvalValue::Binary(bits.clone()),
        P21Value::Str(text) => {
            if let Some(target) = text.strip_prefix('#') {
                match target
                    .parse::<u64>()
                    .ok()
                    .and_then(|id| ctx.instance_for(id))
                {
                    Some(record) => EvalValue::Instance {
                        id: target.parse().unwrap_or(0),
                        record,
                    },
                    None => EvalValue::Str(text.clone()),
                }
            } else {
                EvalValue::Str(text.clone())
            }
        }
        P21Value::Ref(id) => match ctx.instance_for(*id) {
            Some(record) => EvalValue::Instance { id: *id, record },
            None => EvalValue::Str(format!("#{}", id)),
        },
        P21Value::Number(text) => {
            if let Ok(value) = text.parse::<i64>() {
                EvalValue::Int(value)
            } else {
                EvalValue::Float(text.parse().unwrap_or(0.0))
            }
        }
        P21Value::Constant(text) => EvalValue::Str(text.clone()),
        P21Value::Typed(typed) => EvalValue::Aggregate {
            kind: AggKind::List,
            elements: typed
                .params
                .iter()
                .map(|param| instance_value(param, ctx))
                .collect::<Result<Vec<_>, _>>()?,
        },
        P21Value::List(items) => EvalValue::Aggregate {
            kind: AggKind::List,
            elements: items
                .iter()
                .map(|item| instance_value(item, ctx))
                .collect::<Result<Vec<_>, _>>()?,
        },
    })
}

pub fn values_equal(one: &EvalValue, other: &EvalValue) -> bool {
    match (one, other) {
        (EvalValue::Enumeration(a), EvalValue::Enumeration(b)) => a == b,
        (EvalValue::Logical(a), EvalValue::Logical(b)) => a == b,
        (EvalValue::Binary(a), EvalValue::Binary(b)) => a == b,
        (EvalValue::Aggregate { elements: a, .. }, EvalValue::Aggregate { elements: b, .. }) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| values_equal(x, y))
        }
        (a, b) => a == b,
    }
}

fn evaluate_in(element: EvalValue, aggregate_value: EvalValue) -> Result<EvalValue, EvalError> {
    if matches!(element, EvalValue::Indeterminate)
        || matches!(aggregate_value, EvalValue::Indeterminate)
    {
        return Ok(EvalValue::Logical(Logical::Unknown));
    }
    let EvalValue::Aggregate { elements, .. } = &aggregate_value else {
        return err("membership requires an aggregate right operand (12.2.3)");
    };
    if elements.iter().any(|e| values_equal(&element, e)) {
        return Ok(EvalValue::Logical(Logical::True));
    }
    if elements
        .iter()
        .any(|e| matches!(e, EvalValue::Indeterminate))
    {
        return Ok(EvalValue::Logical(Logical::Unknown));
    }
    Ok(EvalValue::Logical(Logical::False))
}

fn describe(value: &EvalValue) -> &'static str {
    match value {
        EvalValue::Logical(_) => "LOGICAL",
        EvalValue::Enumeration(_) => "ENUMERATION",
        EvalValue::Binary(_) => "BINARY",
        EvalValue::Aggregate { .. } => "aggregate",
        EvalValue::Str(_) => "string",
        EvalValue::Int(_) | EvalValue::Float(_) => "number",
        EvalValue::Indeterminate => "?",
        EvalValue::Instance { .. } => "instance",
    }
}

impl EvalValue {
    fn to_float(&self) -> Result<f64, EvalError> {
        match self {
            EvalValue::Int(i) => Ok(*i as f64),
            EvalValue::Float(f) => Ok(*f),
            other => err(format!(
                "expected a numeric operand, got {}",
                describe(other)
            )),
        }
    }
}
