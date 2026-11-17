//! WHERE-rule checking over a Part 21 population (expressir#639,
//! slice 3) — a port of the ruby `Expressir::Eval::Rules` (ISO
//! 10303-11 9.4: a rule is violated only when it evaluates to false;
//! unknown/indeterminate is not a violation). `check_json` is the
//! delegation entry: the schema wire + the gate population encoding
//! in, the RuleResult sequence out (source order, then rule order).
//!
//! The population-scale engine (`Eval::Population` — typing, UNIQUE,
//! global rules) is the next slice.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::eval::{evaluate, instance_attribute, Ctx, Env, EvalError, EvalValue, Logical};
use crate::population::{Instance, P21Value, Record};

fn err<T>(message: impl Into<String>) -> Result<T, EvalError> {
    Err(EvalError(message.into()))
}

/// The gate population encoding: the ruby P21 parse reduced to the
/// records the evaluator consumes; params carry their P21 kinds.
#[derive(Debug, Deserialize, Serialize)]
struct GatePopulation {
    instances: Vec<GateInstance>,
    #[serde(default)]
    mutations: Vec<Value>,
}

#[derive(Debug, Deserialize, Serialize)]
struct GateInstance {
    id: u64,
    records: Vec<GateRecord>,
}

#[derive(Debug, Deserialize, Serialize)]
struct GateRecord {
    keyword: String,
    params: Vec<Value>,
}

fn decode_param(wire: &Value) -> Result<P21Value, String> {
    if let Some(text) = wire.as_str() {
        return match text {
            "unset" => Ok(P21Value::Unset),
            "derived" => Ok(P21Value::Derived),
            other => Ok(P21Value::Str(other.to_string())),
        };
    }
    let object = wire.as_object().ok_or("param encoding must be tagged")?;
    if let Some(flag) = object.get("bool") {
        return Ok(P21Value::Bool(flag.as_bool().ok_or("bad bool")?));
    }
    if let Some(name) = object.get("enum") {
        return Ok(P21Value::Enum(name.as_str().ok_or("bad enum")?.to_string()));
    }
    if let Some(bits) = object.get("binary") {
        return Ok(P21Value::Binary(
            bits.as_str().ok_or("bad binary")?.to_string(),
        ));
    }
    if let Some(id) = object.get("ref") {
        return Ok(P21Value::Ref(id.as_u64().ok_or("bad ref")?));
    }
    if let Some(text) = object.get("number") {
        return Ok(P21Value::Number(
            text.as_str().ok_or("bad number")?.to_string(),
        ));
    }
    if let Some(text) = object.get("const") {
        return Ok(P21Value::Constant(
            text.as_str().ok_or("bad const")?.to_string(),
        ));
    }
    if let Some(typed) = object.get("typed") {
        let keyword = typed
            .get("keyword")
            .and_then(Value::as_str)
            .ok_or("bad typed keyword")?
            .to_string();
        let params = typed
            .get("params")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(decode_param)
                    .collect::<Result<Vec<_>, _>>()
            })
            .unwrap_or(Ok(Vec::new()))?;
        return Ok(P21Value::Typed(crate::population::Typed {
            keyword,
            params,
        }));
    }
    if let Some(items) = object.get("list") {
        let elements = items
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(decode_param)
                    .collect::<Result<Vec<_>, _>>()
            })
            .unwrap_or(Ok(Vec::new()))?;
        return Ok(P21Value::List(elements));
    }
    Err("unsupported param encoding".to_string())
}

fn decode_population(
    wire: &Value,
) -> Result<(crate::population::FullPopulation, Vec<Value>), String> {
    let gate: GatePopulation =
        serde_json::from_value(wire.clone()).map_err(|e| format!("population json: {e}"))?;
    let mutations = gate.mutations.clone();
    let instances = gate
        .instances
        .into_iter()
        .map(|instance| {
            let record = if instance.records.len() == 1 {
                let gate = &instance.records[0];
                Instance::Simple(Record {
                    keyword: gate.keyword.clone(),
                    params: gate
                        .params
                        .iter()
                        .map(decode_param)
                        .collect::<Result<Vec<_>, _>>()?,
                })
            } else {
                Instance::Complex(
                    instance
                        .records
                        .iter()
                        .map(|gate| {
                            Ok(Record {
                                keyword: gate.keyword.clone(),
                                params: gate
                                    .params
                                    .iter()
                                    .map(decode_param)
                                    .collect::<Result<Vec<_>, _>>()?,
                            })
                        })
                        .collect::<Result<Vec<_>, String>>()?,
                )
            };
            Ok((instance.id, record))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok((
        crate::population::FullPopulation {
            instances,
            ..Default::default()
        },
        mutations,
    ))
}

fn apply_mutation(
    population: &mut crate::population::FullPopulation,
    mutation: &Value,
) -> Result<(), String> {
    let target = mutation
        .get("target")
        .and_then(Value::as_u64)
        .ok_or("mutation target")?;
    let index = mutation
        .get("index")
        .and_then(Value::as_u64)
        .ok_or("mutation index")? as usize;
    let value = decode_param(mutation.get("value").ok_or("mutation value")?)?;
    let Some((_, record)) = population
        .instances
        .iter_mut()
        .find(|(id, _)| *id == target)
    else {
        return Err(format!("#{target}: no such instance"));
    };
    let record = match record {
        Instance::Simple(record) => record,
        Instance::Complex(records) => records.first_mut().ok_or("complex record without parts")?,
    };
    if index >= record.params.len() {
        return Err(format!("#{target}: param index {index} out of range"));
    }
    record.params[index] = value;
    Ok(())
}

/// The ruby Rules.entity_for: a complex record takes its first part.
fn entity_for<'a>(ctx: &'a Ctx, record: &Instance) -> Option<&'a Value> {
    let keyword = match record {
        Instance::Simple(record) => &record.keyword,
        Instance::Complex(records) => &records.first()?.keyword,
    };
    ctx.entity_for_keyword(keyword)
}

fn environment_for(
    id: u64,
    record: &Instance,
    entity: &Value,
    ctx: &Ctx,
) -> Result<Env, EvalError> {
    let mut env = Env::with_variables_ctx(Vec::new(), ctx.clone());
    let instance = EvalValue::Instance {
        id,
        record: record.clone(),
    };
    env.define("self", instance.clone());
    let EvalValue::Instance { record, .. } = &instance else {
        return err("self bound to a non-instance");
    };
    for (name, _) in ctx.slots_for(entity) {
        let value = instance_attribute(record, &name, ctx)?;
        env.define(&name, value);
    }
    Ok(env)
}

fn status_of(value: &EvalValue) -> &'static str {
    match value {
        EvalValue::Indeterminate => "unknown",
        EvalValue::Logical(Logical::True) => "pass",
        EvalValue::Logical(Logical::False) => "fail",
        _ => "unknown",
    }
}

/// The delegation entry: schema wire + population gate encoding +
/// mutations in, the RuleResult sequence out.
pub fn check_json(schema_json: &str, population_json: &str) -> Result<String, String> {
    let schema: Value =
        serde_json::from_str(schema_json).map_err(|e| format!("schema json: {e}"))?;
    let (mut population, mutations) = decode_population(
        &serde_json::from_str(population_json).map_err(|e| format!("population json: {e}"))?,
    )?;
    for mutation in &mutations {
        apply_mutation(&mut population, mutation)?;
    }
    let ctx = Ctx::from_schema(&schema).with_population(population);
    let mut results: Vec<Value> = Vec::new();
    let Some(population) = ctx.population() else {
        return serde_json::to_string(&json!({ "results": results }))
            .map_err(|e| format!("result json: {e}"));
    };
    for (id, record) in &population.instances {
        let Some(entity) = entity_for(&ctx, record) else {
            continue;
        };
        let entity_id = entity.get("id").and_then(Value::as_str).unwrap_or("");
        for rule in entity
            .get("where_rules")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[])
        {
            let rule_id = rule.get("id").and_then(Value::as_str).unwrap_or("");
            let outcome = (|| -> Result<Value, EvalError> {
                let mut env = environment_for(*id, record, entity, &ctx)?;
                let value = evaluate(rule.get("expression").unwrap_or(&Value::Null), &mut env)?;
                let status = status_of(&value);
                let message = if status == "fail" {
                    format!("#{id}: {entity_id}.{rule_id}")
                } else {
                    String::new()
                };
                Ok(json!({
                    "instance": format!("#{id}"),
                    "rule_id": rule_id,
                    "status": status,
                    "message": if message.is_empty() { Value::Null } else { json!(message) },
                }))
            })();
            match outcome {
                Ok(result) => results.push(result),
                Err(error) => results.push(json!({
                    "instance": format!("#{id}"),
                    "rule_id": rule_id,
                    "status": "error",
                    "message": format!("#{id}: {entity_id}.{rule_id}: {}", error.0),
                })),
            }
        }
    }
    serde_json::to_string(&json!({ "results": results })).map_err(|e| format!("result json: {e}"))
}

// ---------------------------------------------------------------------
// The population-scale engine (the ruby Eval::Population)
// ---------------------------------------------------------------------

/// Typing entities for a record: the keyword entity plus its
/// supertype closure, per part (the ruby P21::Typing#of entity list).
fn typing_entities<'a>(ctx: &'a Ctx, record: &Instance) -> Vec<&'a Value> {
    let parts: Vec<&str> = match record {
        Instance::Simple(record) => vec![&record.keyword],
        Instance::Complex(records) => records.iter().map(|r| r.keyword.as_str()).collect(),
    };
    let mut entities: Vec<&Value> = Vec::new();
    for keyword in parts {
        let Some(entity) = ctx.entity_for_keyword(keyword) else {
            continue;
        };
        entities.push(entity);
        for supertype in crate::eval::supertype_closure(ctx, entity) {
            if !entities.iter().any(|e| e.get("id") == supertype.get("id")) {
                entities.push(supertype);
            }
        }
    }
    entities
}

struct Finding {
    kind: &'static str,
    instance: Option<u64>,
    rule_id: String,
    message: String,
}

fn finding_json(finding: &Finding) -> Value {
    json!({
        "kind": finding.kind,
        "instance": finding
            .instance
            .map(|id| json!(format!("#{id}")))
            .unwrap_or(Value::Null),
        "rule_id": finding.rule_id,
        "message": finding.message,
    })
}

/// The instances conforming to the named entity, including subtypes,
/// in population source order.
fn instances_for<'a>(
    by_entity: &'a [(String, Vec<(u64, Instance)>)],
    entity: &Value,
) -> &'a [(u64, Instance)] {
    let key = entity
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase();
    by_entity
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, instances)| instances.as_slice())
        .unwrap_or(&[])
}

fn value_key(value: &EvalValue) -> String {
    serde_json::to_string(&crate::eval::value_to_json(value)).unwrap_or_default()
}

fn false_value(value: &EvalValue) -> bool {
    matches!(value, EvalValue::Logical(Logical::False))
}

/// The ruby Eval::Population#check: where + unique + global findings.
pub fn population_check(
    schema: &Value,
    population: crate::population::FullPopulation,
) -> Result<Vec<Value>, String> {
    let ctx = Ctx::from_schema(schema).with_population(population);
    let Some(loaded) = ctx.population() else {
        return Ok(Vec::new());
    };
    let instances: Vec<(u64, Instance)> = loaded.instances.clone();

    let mut by_entity: Vec<(String, Vec<(u64, Instance)>)> = Vec::new();
    for (id, record) in &instances {
        for entity in typing_entities(&ctx, record) {
            let key = entity
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_lowercase();
            match by_entity.iter_mut().find(|(name, _)| *name == key) {
                Some((_, bucket)) => bucket.push((*id, record.clone())),
                None => by_entity.push((key, vec![(*id, record.clone())])),
            }
        }
    }

    let mut findings: Vec<Finding> = Vec::new();

    // WHERE rules (9.2.1 via typing, 9.4 false-only violation)
    for entity in ctx_entities(&ctx) {
        let rules = entity
            .get("where_rules")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if rules.is_empty() {
            continue;
        }
        for (id, record) in instances_for(&by_entity, entity) {
            for rule in &rules {
                let rule_id = rule.get("id").and_then(Value::as_str).unwrap_or("");
                let outcome = (|| -> Result<Option<Finding>, EvalError> {
                    let mut env = environment_for(*id, record, entity, &ctx)?;
                    let value = evaluate(rule.get("expression").unwrap_or(&Value::Null), &mut env)?;
                    if false_value(&value) {
                        return Ok(Some(Finding {
                            kind: "where",
                            instance: Some(*id),
                            rule_id: rule_id.to_string(),
                            message: format!(
                                "#{id}: {}.{} (9.4)",
                                entity.get("id").and_then(Value::as_str).unwrap_or(""),
                                rule_id
                            ),
                        }));
                    }
                    Ok(None)
                })();
                match outcome {
                    Ok(Some(finding)) => findings.push(finding),
                    Ok(None) => {}
                    Err(error) => findings.push(Finding {
                        kind: "where",
                        instance: Some(*id),
                        rule_id: rule_id.to_string(),
                        message: format!(
                            "#{id}: {}.{}: {}",
                            entity.get("id").and_then(Value::as_str).unwrap_or(""),
                            rule_id,
                            error.0
                        ),
                    }),
                }
            }
        }
    }

    // UNIQUE rules (9.2.2.1): the value tuple across the entity's
    // instances and subtypes.
    for entity in ctx_entities(&ctx) {
        let entity_id = entity.get("id").and_then(Value::as_str).unwrap_or("");
        for rule in entity
            .get("unique_rules")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[])
        {
            let rule_id = rule.get("id").and_then(Value::as_str).unwrap_or("");
            let mut seen: Vec<(String, u64)> = Vec::new();
            for (id, record) in instances_for(&by_entity, entity) {
                let mut env = match environment_for(*id, record, entity, &ctx) {
                    Ok(env) => env,
                    Err(error) => {
                        findings.push(Finding {
                            kind: "unique",
                            instance: Some(*id),
                            rule_id: rule_id.to_string(),
                            message: format!("#{id}: UNIQUE '{rule_id}': {}", error.0),
                        });
                        continue;
                    }
                };
                let mut key = String::new();
                let mut outside = false;
                for reference in rule
                    .get("attributes")
                    .and_then(Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or(&[])
                {
                    let name = reference.get("id").and_then(Value::as_str).unwrap_or("");
                    if !ctx
                        .slots_for(entity)
                        .iter()
                        .any(|(slot, _)| slot.as_str() == name.to_lowercase())
                    {
                        outside = true;
                        break;
                    }
                    let EvalValue::Instance { record, .. } = env
                        .lookup("self")
                        .cloned()
                        .unwrap_or(EvalValue::Indeterminate)
                    else {
                        outside = true;
                        break;
                    };
                    match instance_attribute(&record, name, &ctx) {
                        Ok(value) => key.push_str(&value_key(&value)),
                        Err(_) => {
                            outside = true;
                            break;
                        }
                    }
                }
                let _ = &mut env;
                if outside {
                    findings.push(Finding {
                        kind: "unique",
                        instance: Some(*id),
                        rule_id: rule_id.to_string(),
                        message: format!(
                            "#{id}: UNIQUE '{rule_id}' references an attribute outside '{entity_id}' (9.2.2.1)"
                        ),
                    });
                    continue;
                }
                match seen.iter().find(|(seen_key, _)| *seen_key == key) {
                    Some((_, first)) => findings.push(Finding {
                        kind: "unique",
                        instance: Some(*id),
                        rule_id: rule_id.to_string(),
                        message: format!("#{id}: UNIQUE '{rule_id}' duplicates #{first} (9.2.2.1)"),
                    }),
                    None => seen.push((key, *id)),
                }
            }
        }
    }

    // Global rules (9.4): the FOR entities bound to their aggregates.
    for rule in schema
        .get("rules")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        let rule_id = rule.get("id").and_then(Value::as_str).unwrap_or("");
        let mut env = Env::with_variables_ctx(Vec::new(), ctx.clone());
        let mut bound = 0usize;
        let mut named = true;
        for reference in rule
            .get("applies_to")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[])
        {
            let name = reference
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let Some(entity) = ctx.entity_for_keyword(&name) else {
                named = false;
                break;
            };
            let members: Vec<EvalValue> = instances_for(&by_entity, entity)
                .iter()
                .map(|(id, record)| EvalValue::Instance {
                    id: *id,
                    record: record.clone(),
                })
                .collect();
            env.define(
                &name,
                EvalValue::Aggregate {
                    kind: crate::eval::AggKind::List,
                    elements: members,
                },
            );
            bound += 1;
        }
        if !named || bound == 0 {
            findings.push(Finding {
                kind: "global",
                instance: None,
                rule_id: rule_id.to_string(),
                message: format!("global rule '{rule_id}' names no entity (9.4)"),
            });
            continue;
        }
        for where_rule in rule
            .get("where_rules")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[])
        {
            let outcome = (|| -> Result<Option<Finding>, EvalError> {
                let value = evaluate(
                    where_rule.get("expression").unwrap_or(&Value::Null),
                    &mut env,
                )?;
                if false_value(&value) {
                    return Ok(Some(Finding {
                        kind: "global",
                        instance: None,
                        rule_id: rule_id.to_string(),
                        message: format!("global rule '{rule_id}' (9.4)"),
                    }));
                }
                Ok(None)
            })();
            match outcome {
                Ok(Some(finding)) => findings.push(finding),
                Ok(None) => {}
                Err(error) => findings.push(Finding {
                    kind: "global",
                    instance: None,
                    rule_id: rule_id.to_string(),
                    message: format!("global rule '{rule_id}': {}", error.0),
                }),
            }
        }
    }

    Ok(findings.iter().map(finding_json).collect())
}

fn ctx_entities(ctx: &Ctx) -> &[Value] {
    ctx.entities_ref()
}

/// The delegation entry for the population-scale engine: the schema
/// wire and the population gate encoding (mutations included) in,
/// the Finding sequence out (where findings, then unique, then
/// global).
pub fn population_check_json(schema_json: &str, population_json: &str) -> Result<String, String> {
    let schema: Value =
        serde_json::from_str(schema_json).map_err(|e| format!("schema json: {e}"))?;
    let (mut population, mutations) = decode_population(
        &serde_json::from_str(population_json).map_err(|e| format!("population json: {e}"))?,
    )?;
    for mutation in &mutations {
        apply_mutation(&mut population, mutation)?;
    }
    let findings = population_check(&schema, population)?;
    serde_json::to_string(&json!({ "findings": findings })).map_err(|e| format!("result json: {e}"))
}
