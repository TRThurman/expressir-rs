//! The interpreter (expressir#639, slice 2): ISO 10303-11 ch.13
//! statements over the model-hash wire — a port of the ruby
//! `Expressir::Eval::Interpreter` statement half plus the schema-
//! function override of `Interpreter#call`. `call_json` is the
//! delegation entry; `call_function` is shared with the expression
//! lane's call override in `eval.rs`.
//!
//! Known slice-2 deviations (ungated): a beyond-end indexed
//! assignment extends with the indeterminate value where ruby nil
//! would sit, an uncaught ESCAPE/SKIP errors instead of raising
//! ruby's UncaughtThrowError, and a bare RETURN nested in an
//! expression call becomes indeterminate. The P21 population faces
//! (instance assignment, SELF, TYPEOF/USEDIN over instances) arrive
//! with the population slice.

use serde_json::{json, Value};

use crate::eval::{
    class_of, evaluate, value_from_json, value_to_json, values_equal, Ctx, Env, EvalError,
    EvalValue, Logical,
};

fn err<T>(message: impl Into<String>) -> Result<T, EvalError> {
    Err(EvalError(message.into()))
}

fn is(node: &Value, tail: &str) -> bool {
    class_of(node).ends_with(&format!("::{tail}"))
}

/// Statement control flow: the ruby throws (`eval_return`,
/// `eval_escape`, `eval_skip`) as an explicit result.
enum Flow {
    Continue,
    Escape,
    Skip,
    Return(Option<EvalValue>),
}

/// The delegation entry: a schema wire node + a function name + JSON
/// arguments in, the result value out (a bare RETURN encodes as
/// `{"nil": true}`).
pub fn call_json(schema_json: &str, name: &str, arguments_json: &str) -> Result<String, String> {
    let schema: Value =
        serde_json::from_str(schema_json).map_err(|e| format!("schema json: {e}"))?;
    let ctx = Ctx::from_schema(&schema);
    let arguments: Vec<Value> =
        serde_json::from_str(arguments_json).map_err(|e| format!("arguments json: {e}"))?;
    let values = arguments
        .iter()
        .map(value_from_json)
        .collect::<Result<Vec<_>, _>>()?;
    let result = if let Some(function) = ctx.find_function(name) {
        call_function(function, values, &ctx).map_err(|e| e.0)?
    } else {
        // The ruby interpreter falls through to the builtin table.
        Some(crate::eval_builtins::call_builtin(name, &values, Some(&ctx)).map_err(|e| e.0)?)
    };
    let encoded = match result {
        Some(value) => value_to_json(&value),
        None => json!({ "nil": true }),
    };
    serde_json::to_string(&encoded).map_err(|e| format!("result json: {e}"))
}

/// The ruby `Interpreter#execute_function`: a fresh scope seeded with
/// the parameters, the locals initialized, the body run to RETURN.
pub fn call_function(
    function: &Value,
    arguments: Vec<EvalValue>,
    ctx: &Ctx,
) -> Result<Option<EvalValue>, EvalError> {
    let mut env = Env::with_variables_ctx(Vec::new(), ctx.clone());
    let parameters = function
        .get("parameters")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for (index, parameter) in parameters.iter().enumerate() {
        let name = parameter.get("id").and_then(Value::as_str).unwrap_or("");
        let value = arguments
            .get(index)
            .cloned()
            .unwrap_or(EvalValue::Indeterminate);
        env.define(name, value);
    }
    let variables = function
        .get("variables")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for variable in &variables {
        let name = variable.get("id").and_then(Value::as_str).unwrap_or("");
        let value = match variable.get("expression") {
            Some(expression) if !expression.is_null() => evaluate(expression, &mut env)?,
            _ => EvalValue::Indeterminate,
        };
        env.define(name, value);
    }
    let statements = function
        .get("statements")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    match execute_statements(&statements, &mut env)? {
        Flow::Return(value) => Ok(value),
        _ => err(format!(
            "function '{}' ended without RETURN (13.7)",
            function.get("id").and_then(Value::as_str).unwrap_or("")
        )),
    }
}

fn execute_statements(statements: &[Value], env: &mut Env) -> Result<Flow, EvalError> {
    for statement in statements {
        match execute_statement(statement, env)? {
            Flow::Continue => {}
            other => return Ok(other),
        }
    }
    Ok(Flow::Continue)
}

fn execute_statement(statement: &Value, env: &mut Env) -> Result<Flow, EvalError> {
    match class_of(statement).rsplit("::").next().unwrap_or("") {
        "Null" => Ok(Flow::Continue),
        "Assignment" => execute_assignment(statement, env),
        "If" => execute_if(statement, env),
        "Case" => execute_case(statement, env),
        "Repeat" => execute_repeat(statement, env),
        "Return" => {
            let value = match statement.get("expression") {
                Some(expression) if !expression.is_null() => Some(evaluate(expression, env)?),
                _ => None,
            };
            Ok(Flow::Return(value))
        }
        "Escape" => Ok(Flow::Escape),
        "Skip" => Ok(Flow::Skip),
        "ProcedureCall" => err("procedure calls arrive with TODO.sota/02 step 3+"),
        "Alias" => err("ALIAS statements arrive with TODO.sota/02 step 3+"),
        _other => err(format!("unsupported statement {}", class_of(statement))),
    }
}

fn execute_assignment(statement: &Value, env: &mut Env) -> Result<Flow, EvalError> {
    let value = evaluate(statement.get("expression").unwrap_or(&Value::Null), env)?;
    let target = statement.get("ref").unwrap_or(&Value::Null);
    if is(target, "SimpleReference") {
        let name = target.get("id").and_then(Value::as_str).unwrap_or("");
        env.define(name, value);
        return Ok(Flow::Continue);
    }
    if is(target, "IndexReference") {
        return execute_indexed_assignment(target, value, env);
    }
    err("assignment to instance attributes requires a population (step 3)")
}

fn execute_indexed_assignment(
    target: &Value,
    value: EvalValue,
    env: &mut Env,
) -> Result<Flow, EvalError> {
    let index2 = target.get("index2");
    if index2.is_some_and(|i| !i.is_null()) {
        return err("aggregate element assignment expects one index");
    }
    let base = target.get("ref").unwrap_or(&Value::Null);
    if !is(base, "SimpleReference") {
        return err("nested indexed assignment requires a population (step 3)");
    }
    let name = base.get("id").and_then(Value::as_str).unwrap_or("");
    let aggregate = match env.lookup(name) {
        Some(EvalValue::Aggregate { kind, elements }) => (*kind, elements.clone()),
        Some(_) => return err("indexed assignment requires an aggregate variable"),
        None => return err("indexed assignment requires an aggregate variable"),
    };
    let index = match evaluate(target.get("index1").unwrap_or(&Value::Null), env)? {
        EvalValue::Int(i) => i - 1,
        _ => return err("expected a numeric operand, got index"),
    };
    let len = aggregate.1.len() as i64;
    let resolved = if index < 0 {
        // Ruby negative indexing writes from the end; past the front
        // raises IndexError.
        let from_end = len + index;
        if from_end < 0 {
            return err("index out of range");
        }
        from_end
    } else {
        index
    };
    let mut elements = aggregate.1;
    let slot = resolved as usize;
    if slot >= elements.len() {
        elements.resize(slot + 1, EvalValue::Indeterminate);
    }
    elements[slot] = value;
    env.define(
        name,
        EvalValue::Aggregate {
            kind: aggregate.0,
            elements,
        },
    );
    Ok(Flow::Continue)
}

fn truth(condition: &EvalValue) -> Logical {
    match condition {
        EvalValue::Logical(logical) => *logical,
        _ => Logical::Unknown,
    }
}

/// 12.4: only TRUE selects the then-branch.
fn execute_if(statement: &Value, env: &mut Env) -> Result<Flow, EvalError> {
    let condition = evaluate(statement.get("expression").unwrap_or(&Value::Null), env)?;
    let branch = if truth(&condition) == Logical::True {
        statement.get("statements")
    } else {
        statement.get("else_statements")
    };
    match branch.and_then(Value::as_array) {
        Some(statements) => execute_statements(statements, env),
        None => Ok(Flow::Continue),
    }
}

fn execute_case(statement: &Value, env: &mut Env) -> Result<Flow, EvalError> {
    let selector = evaluate(statement.get("expression").unwrap_or(&Value::Null), env)?;
    let mut body: Option<&Value> = None;
    'actions: for action in statement
        .get("actions")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        for label in action
            .get("labels")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[])
        {
            let value = evaluate(label, env)?;
            if values_equal(&selector, &value) {
                body = action.get("statement");
                break 'actions;
            }
        }
    }
    let body = match body.or(statement.get("otherwise_statement")) {
        Some(body) if !body.is_null() => body,
        _ => return Ok(Flow::Continue),
    };
    execute_statement(body, env)
}

fn execute_repeat(statement: &Value, env: &mut Env) -> Result<Flow, EvalError> {
    if statement
        .get("while_expression")
        .is_some_and(|w| !w.is_null())
    {
        execute_while(statement, env)
    } else if statement
        .get("until_expression")
        .is_some_and(|u| !u.is_null())
    {
        execute_until(statement, env)
    } else {
        execute_increment(statement, env)
    }
}

fn execute_while(statement: &Value, env: &mut Env) -> Result<Flow, EvalError> {
    let statements = statement
        .get("statements")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    loop {
        let condition = evaluate(
            statement.get("while_expression").unwrap_or(&Value::Null),
            env,
        )?;
        if truth(&condition) != Logical::True {
            return Ok(Flow::Continue);
        }
        match execute_statements(&statements, env)? {
            Flow::Continue => {}
            Flow::Escape => return Ok(Flow::Continue),
            // SKIP without an increment control has no catcher in the
            // ruby lane either (UncaughtThrowError).
            Flow::Skip => return err("uncaught throw :eval_skip"),
            flow @ Flow::Return(_) => return Ok(flow),
        }
    }
}

fn execute_until(statement: &Value, env: &mut Env) -> Result<Flow, EvalError> {
    let statements = statement
        .get("statements")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    loop {
        match execute_statements(&statements, env)? {
            Flow::Continue => {}
            Flow::Escape => return Ok(Flow::Continue),
            Flow::Skip => return err("uncaught throw :eval_skip"),
            flow @ Flow::Return(_) => return Ok(flow),
        }
        let condition = evaluate(
            statement.get("until_expression").unwrap_or(&Value::Null),
            env,
        )?;
        if truth(&condition) == Logical::True {
            return Ok(Flow::Continue);
        }
    }
}

fn execute_increment(statement: &Value, env: &mut Env) -> Result<Flow, EvalError> {
    let first = match evaluate(statement.get("bound1").unwrap_or(&Value::Null), env)? {
        EvalValue::Int(i) => i,
        _ => return err("expected a numeric operand, got bound"),
    };
    let last = match evaluate(statement.get("bound2").unwrap_or(&Value::Null), env)? {
        EvalValue::Int(i) => i,
        _ => return err("expected a numeric operand, got bound"),
    };
    let step = match statement.get("increment") {
        Some(increment) if !increment.is_null() => match evaluate(increment, env)? {
            EvalValue::Int(i) => i,
            _ => return err("expected a numeric operand, got bound"),
        },
        _ => 1,
    };
    if step == 0 {
        return err("repeat increment must not be zero (13.6.1)");
    }
    let control = statement.get("id").and_then(Value::as_str).unwrap_or("");
    let statements = statement
        .get("statements")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut value = first;
    loop {
        if step > 0 && value > last {
            return Ok(Flow::Continue);
        }
        if step < 0 && value < last {
            return Ok(Flow::Continue);
        }
        env.define(control, EvalValue::Int(value));
        match execute_statements(&statements, env)? {
            Flow::Continue => {}
            // The env at ESCAPE/SKIP wins — the mutable chain already
            // carries the changes made so far.
            Flow::Escape => return Ok(Flow::Continue),
            Flow::Skip => {}
            flow @ Flow::Return(_) => return Ok(flow),
        }
        value += step;
    }
}
