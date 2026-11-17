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
