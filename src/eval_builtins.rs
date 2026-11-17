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
pub fn call_builtin(name: &str, arguments: &[EvalValue]) -> Result<EvalValue, EvalError> {
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
            // Slice 1: instances arrive with the population slice.
            EvalValue::Aggregate { .. } => Err(EvalError("TYPEOF requires a population".into())),
            _ => Ok(EvalValue::Aggregate {
                kind: AggKind::Set,
                elements: Vec::new(),
            }),
        },
        "USEDIN" => Ok(EvalValue::Aggregate {
            kind: AggKind::Bag,
            elements: Vec::new(),
        }),
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

/// The delegation entry: builtin name + JSON arguments in, the JSON
/// value out.
pub fn call_builtin_json(name: &str, arguments_json: &str) -> Result<String, String> {
    let arguments: Vec<Value> =
        serde_json::from_str(arguments_json).map_err(|e| format!("arguments json: {e}"))?;
    let values = arguments
        .iter()
        .map(crate::eval::value_from_json)
        .collect::<Result<Vec<_>, _>>()?;
    let result = call_builtin(name, &values).map_err(|e| e.0)?;
    serde_json::to_string(&value_to_json(&result)).map_err(|e| format!("result json: {e}"))
}
