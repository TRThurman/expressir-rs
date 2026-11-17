//! The pure builtin functions (ISO 10303-11 ch.15 set A) — a port of
//! the ruby `Interpreter#dispatch_builtin` slice-1 table. Instance-
//! aware builtins (TYPEOF/USEDIN over instances) arrive with the
//! population slice; today they return the ruby slice-1 answers
//! (TYPEOF of a non-instance = the empty set, USEDIN = the empty bag).

use serde_json::Value;

use crate::eval::{value_to_json, AggKind, EvalError, EvalValue, Logical};

fn to_number(value: &EvalValue) -> Result<f64, EvalError> {
    match value {
        EvalValue::Int(i) => Ok(*i as f64),
        EvalValue::Float(f) => Ok(*f),
        other => Err(EvalError(format!(
            "expected a numeric operand, got {}",
            describe(other)
        ))),
    }
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

fn elements(value: &EvalValue) -> Result<&Vec<EvalValue>, EvalError> {
    match value {
        EvalValue::Aggregate { elements, .. } => Ok(elements),
        other => Err(EvalError(format!(
            "expected an aggregate operand, got {}",
            describe(other)
        ))),
    }
}

fn logical(flag: bool) -> EvalValue {
    EvalValue::Logical(if flag { Logical::True } else { Logical::False })
}

/// The slice-1 builtin table; the schema-function override arrives
/// with the interpreter slice.
pub fn call_builtin(
    name: &str,
    arguments: &[EvalValue],
    ctx: Option<&crate::eval::Ctx>,
) -> Result<EvalValue, EvalError> {
    let upper = name.to_uppercase();
    let arg = |i: usize| -> Result<&EvalValue, EvalError> {
        arguments
            .get(i)
            .ok_or_else(|| EvalError(format!("builtin '{upper}' missing argument {i}")))
    };
    match upper.as_str() {
        "ABS" => match arg(0)? {
            EvalValue::Int(i) => Ok(EvalValue::Int(i.abs())),
            EvalValue::Float(f) => Ok(EvalValue::Float(f.abs())),
            other => Err(EvalError(format!(
                "expected a numeric operand, got {}",
                describe(other)
            ))),
        },
        "SIN" => Ok(EvalValue::Float(to_number(arg(0)?)?.sin())),
        "COS" => Ok(EvalValue::Float(to_number(arg(0)?)?.cos())),
        "TAN" => Ok(EvalValue::Float(to_number(arg(0)?)?.tan())),
        "ASIN" => Ok(EvalValue::Float(to_number(arg(0)?)?.asin())),
        "ACOS" => Ok(EvalValue::Float(to_number(arg(0)?)?.acos())),
        "ATAN" => Ok(EvalValue::Float(to_number(arg(0)?)?.atan())),
        "ATAN2" => Ok(EvalValue::Float(
            to_number(arg(0)?)?.atan2(to_number(arg(1)?)?),
        )),
        "SQRT" => Ok(EvalValue::Float(to_number(arg(0)?)?.sqrt())),
        "EXP" => Ok(EvalValue::Float(to_number(arg(0)?)?.exp())),
        "LOG" => Ok(EvalValue::Float(to_number(arg(0)?)?.ln())),
        "LOG2" => Ok(EvalValue::Float(to_number(arg(0)?)?.log2())),
        "LOG10" => Ok(EvalValue::Float(to_number(arg(0)?)?.log10())),
        "ODD" => match arg(0)? {
            EvalValue::Int(i) => Ok(logical(i % 2 != 0)),
            other => Err(EvalError(format!(
                "expected a numeric operand, got {}",
                describe(other)
            ))),
        },
        "LENGTH" => match arg(0)? {
            EvalValue::Str(text) => Ok(EvalValue::Int(text.chars().count() as i64)),
            other => Err(EvalError(format!(
                "LENGTH requires a string, got {}",
                describe(other)
            ))),
        },
        "BLENGTH" => match arg(0)? {
            EvalValue::Binary(bits) => Ok(EvalValue::Int(bits.chars().count() as i64)),
            other => Err(EvalError(format!(
                "BLENGTH requires a binary, got {}",
                describe(other)
            ))),
        },
        "SIZEOF" | "HIINDEX" | "HIBOUND" => Ok(EvalValue::Int(elements(arg(0)?)?.len() as i64)),
        "LOINDEX" | "LOBOUND" => Ok(EvalValue::Int(1)),
        "EXISTS" => Ok(logical(!matches!(arg(0)?, EvalValue::Indeterminate))),
        "TYPEOF" => match arg(0)? {
            EvalValue::Instance { id, record } => {
                let Some(ctx) = ctx else {
                    return Ok(EvalValue::Aggregate {
                        kind: AggKind::Set,
                        elements: Vec::new(),
                    });
                };
                typeof_value(*id, record, ctx)
            }
            _ => Ok(EvalValue::Aggregate {
                kind: AggKind::Set,
                elements: Vec::new(),
            }),
        },
        "USEDIN" => match arg(0)? {
            EvalValue::Instance { id, .. } => {
                let Some(ctx) = ctx else {
                    return Ok(EvalValue::Aggregate {
                        kind: AggKind::Bag,
                        elements: Vec::new(),
                    });
                };
                usedin_value(*id, ctx)
            }
            _ => Ok(EvalValue::Aggregate {
                kind: AggKind::Bag,
                elements: Vec::new(),
            }),
        },
        "NVL" => {
            if matches!(arg(0)?, EvalValue::Indeterminate) {
                Ok(arg(1)?.clone())
            } else {
                Ok(arg(0)?.clone())
            }
        }
        "BOUNDS" => Ok(EvalValue::Aggregate {
            kind: AggKind::List,
            elements: vec![
                EvalValue::Int(1),
                EvalValue::Int(elements(arg(0)?)?.len() as i64),
            ],
        }),
        "ROLESOF" => Ok(EvalValue::Aggregate {
            kind: AggKind::Set,
            elements: Vec::new(),
        }),
        "VALUE_IN" => {
            let candidates = elements(arg(1)?)?;
            let present = candidates.iter().any(|candidate| {
                crate::eval::values_equal(candidate, arg(0).unwrap_or(&EvalValue::Indeterminate))
            });
            Ok(logical(present))
        }
        "VALUE" => match arg(0)? {
            EvalValue::Int(_) | EvalValue::Float(_) => Ok(arg(0)?.clone()),
            EvalValue::Str(text) => {
                if text.trim().parse::<i64>().is_ok() {
                    Ok(EvalValue::Int(text.trim().parse().unwrap()))
                } else {
                    text.trim()
                        .parse::<f64>()
                        .map(EvalValue::Float)
                        .map_err(|_| EvalError("VALUE requires a numeric string".into()))
                }
            }
            other => Err(EvalError(format!(
                "VALUE requires a numeric or string, got {}",
                describe(other)
            ))),
        },
        other => Err(EvalError(format!(
            "builtin '{other}' is not implemented yet"
        ))),
    }
}

/// The ruby Interpreter#typeof_value: the record keywords expanded
/// through the supertype closure, unqualified and schema-qualified.
fn typeof_value(
    id: u64,
    record: &crate::population::Instance,
    ctx: &crate::eval::Ctx,
) -> Result<EvalValue, EvalError> {
    let keywords: Vec<String> = match record {
        crate::population::Instance::Simple(record) => vec![record.keyword.clone()],
        crate::population::Instance::Complex(records) => {
            records.iter().map(|r| r.keyword.clone()).collect()
        }
    };
    let mut names: Vec<String> = Vec::new();
    for keyword in keywords {
        let Some(entity) = ctx.entity_for_keyword(&keyword) else {
            names.push(keyword.to_uppercase());
            continue;
        };
        let mut chain: Vec<&Value> = crate::eval::supertype_closure(ctx, entity);
        chain.insert(0, entity);
        for candidate in chain {
            let name = candidate.get("id").and_then(Value::as_str).unwrap_or("");
            let schema = ctx.schema_id();
            for form in [
                name.to_uppercase(),
                format!("{}.{}", schema.to_uppercase(), name.to_uppercase()),
            ] {
                if !names.contains(&form) {
                    names.push(form);
                }
            }
        }
    }
    let _ = id;
    Ok(EvalValue::Aggregate {
        kind: AggKind::Set,
        elements: names.into_iter().map(EvalValue::Str).collect(),
    })
}

/// The ruby Interpreter#usedin_value: instances whose records
/// reference the target id, as a bag.
fn usedin_value(id: u64, ctx: &crate::eval::Ctx) -> Result<EvalValue, EvalError> {
    let mut referencing: Vec<EvalValue> = Vec::new();
    if let Some(population) = ctx.population() {
        for (instance_id, record) in &population.instances {
            if record_references(record, id) {
                referencing.push(EvalValue::Instance {
                    id: *instance_id,
                    record: record.clone(),
                });
            }
        }
    }
    Ok(EvalValue::Aggregate {
        kind: AggKind::Bag,
        elements: referencing,
    })
}

fn record_references(record: &crate::population::Instance, target: u64) -> bool {
    use crate::population::Instance;
    let records: &[crate::population::Record] = match record {
        Instance::Simple(record) => std::slice::from_ref(record),
        Instance::Complex(records) => records,
    };
    records.iter().any(|record| {
        record
            .params
            .iter()
            .any(|param| params_reference(param, target))
    })
}

fn params_reference(param: &crate::population::P21Value, target: u64) -> bool {
    use crate::population::P21Value;
    match param {
        P21Value::Ref(id) => *id == target,
        P21Value::Str(text) => text.as_str() == format!("#{target}"),
        P21Value::List(items) => items.iter().any(|item| params_reference(item, target)),
        P21Value::Typed(typed) => typed
            .params
            .iter()
            .any(|item| params_reference(item, target)),
        _ => false,
    }
}

/// The delegation entry: builtin name + JSON arguments in, the JSON
/// value out.
pub fn call_builtin_json(name: &str, arguments_json: &str) -> Result<String, String> {
    let arguments: Vec<Value> =
        serde_json::from_str(arguments_json).map_err(|e| format!("arguments json: {e}"))?;
    let values = arguments
        .iter()
        .map(crate::eval::value_from_json)
        .collect::<Result<Vec<_>, _>>()?;
    let result = call_builtin(name, &values, None).map_err(|e| e.0)?;
    serde_json::to_string(&value_to_json(&result)).map_err(|e| format!("result json: {e}"))
}
