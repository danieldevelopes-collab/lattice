//! The built-in function library (M2 starter set). A function takes already
//! evaluated [`Arg`]s — except the *lazy* ones (`IF`, `IFERROR`), which take the
//! raw argument expressions so only the chosen branch is evaluated, exactly as a
//! spreadsheet expects.

use crate::ast::Expr;
use crate::eval::{eval, eval_arg, Arg, Context};
use crate::value::{CellError, Value};

pub fn call(name: &str, arg_exprs: &[Expr], ctx: &dyn Context) -> Value {
    // Lazy functions look at the raw expressions.
    match name {
        "IF" => return f_if(arg_exprs, ctx),
        "IFERROR" => return f_iferror(arg_exprs, ctx),
        _ => {}
    }
    let args: Vec<Arg> = arg_exprs.iter().map(|e| eval_arg(e, ctx)).collect();
    match name {
        "SUM" => num_fold(&args, 0.0, |a, x| a + x),
        "PRODUCT" => num_fold(&args, 1.0, |a, x| a * x),
        "AVERAGE" => f_average(&args),
        "MIN" => f_minmax(&args, true),
        "MAX" => f_minmax(&args, false),
        "COUNT" => f_count(&args, true),
        "COUNTA" => f_count(&args, false),
        "ABS" => map1(&args, |x| Ok(x.abs())),
        "INT" => map1(&args, |x| Ok(x.floor())),
        "SQRT" => map1(&args, |x| if x < 0.0 { Err(CellError::Num) } else { Ok(x.sqrt()) }),
        "ROUND" => f_round(&args),
        "MOD" => f_mod(&args),
        "POWER" => f_power(&args),
        "AND" => f_andor(&args, true),
        "OR" => f_andor(&args, false),
        "NOT" => f_not(&args),
        "LEN" => f_len(&args),
        "UPPER" => f_text1(&args, |s| s.to_uppercase()),
        "LOWER" => f_text1(&args, |s| s.to_lowercase()),
        "TRIM" => f_text1(&args, |s| s.split_whitespace().collect::<Vec<_>>().join(" ")),
        "CONCAT" | "CONCATENATE" => f_concat(&args),
        "TRUE" => Value::Bool(true),
        "FALSE" => Value::Bool(false),
        _ => Value::Error(CellError::Name),
    }
}

// ---- helpers ---------------------------------------------------------------

fn values_of(a: &Arg) -> Vec<&Value> {
    match a {
        Arg::Scalar(v) => vec![v],
        Arg::Multi(vs) => vs.iter().collect(),
    }
}

fn scalar_val(a: &Arg) -> Value {
    match a {
        Arg::Scalar(v) => v.clone(),
        Arg::Multi(vs) => vs.first().cloned().unwrap_or(Value::Empty),
    }
}

fn scalar_num(a: &Arg) -> Result<f64, CellError> {
    scalar_val(a).as_number()
}

/// Collect numbers from arguments with spreadsheet semantics: a direct scalar
/// coerces (numeric text, booleans), while text/booleans/blanks inside a
/// range/reference are ignored; an error anywhere propagates.
fn numbers(args: &[Arg]) -> Result<Vec<f64>, CellError> {
    let mut out = Vec::new();
    for a in args {
        match a {
            Arg::Scalar(v) => match v {
                Value::Number(n) => out.push(*n),
                Value::Bool(b) => out.push(if *b { 1.0 } else { 0.0 }),
                Value::Empty => {}
                Value::Text(s) => out.push(s.trim().parse::<f64>().map_err(|_| CellError::Value)?),
                Value::Error(e) => return Err(*e),
            },
            Arg::Multi(vs) => {
                for v in vs {
                    match v {
                        Value::Number(n) => out.push(*n),
                        Value::Error(e) => return Err(*e),
                        _ => {}
                    }
                }
            }
        }
    }
    Ok(out)
}

fn num_fold(args: &[Arg], init: f64, f: impl Fn(f64, f64) -> f64) -> Value {
    match numbers(args) {
        Ok(ns) => Value::Number(ns.into_iter().fold(init, |a, x| f(a, x))),
        Err(e) => Value::Error(e),
    }
}

fn f_average(args: &[Arg]) -> Value {
    match numbers(args) {
        Ok(ns) if ns.is_empty() => Value::Error(CellError::Div0),
        Ok(ns) => Value::Number(ns.iter().sum::<f64>() / ns.len() as f64),
        Err(e) => Value::Error(e),
    }
}

fn f_minmax(args: &[Arg], min: bool) -> Value {
    match numbers(args) {
        Ok(ns) if ns.is_empty() => Value::Number(0.0),
        Ok(ns) => {
            let v = if min {
                ns.into_iter().fold(f64::INFINITY, f64::min)
            } else {
                ns.into_iter().fold(f64::NEG_INFINITY, f64::max)
            };
            Value::Number(v)
        }
        Err(e) => Value::Error(e),
    }
}

fn f_count(args: &[Arg], numeric_only: bool) -> Value {
    let mut n = 0u64;
    for a in args {
        for v in values_of(a) {
            let hit = if numeric_only {
                matches!(v, Value::Number(_))
            } else {
                !matches!(v, Value::Empty)
            };
            if hit {
                n += 1;
            }
        }
    }
    Value::Number(n as f64)
}

fn map1(args: &[Arg], f: impl Fn(f64) -> Result<f64, CellError>) -> Value {
    if args.len() != 1 {
        return Value::Error(CellError::Value);
    }
    match scalar_num(&args[0]).and_then(f) {
        Ok(r) => Value::Number(r),
        Err(e) => Value::Error(e),
    }
}

fn f_round(args: &[Arg]) -> Value {
    if args.is_empty() || args.len() > 2 {
        return Value::Error(CellError::Value);
    }
    let x = match scalar_num(&args[0]) {
        Ok(v) => v,
        Err(e) => return Value::Error(e),
    };
    let digits = if args.len() == 2 {
        match scalar_num(&args[1]) {
            Ok(v) => v as i32,
            Err(e) => return Value::Error(e),
        }
    } else {
        0
    };
    let f = 10f64.powi(digits);
    Value::Number((x * f).round() / f)
}

fn f_mod(args: &[Arg]) -> Value {
    if args.len() != 2 {
        return Value::Error(CellError::Value);
    }
    let (a, b) = match (scalar_num(&args[0]), scalar_num(&args[1])) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => return Value::Error(e),
    };
    if b == 0.0 {
        return Value::Error(CellError::Div0);
    }
    Value::Number(a - b * (a / b).floor())
}

fn f_power(args: &[Arg]) -> Value {
    if args.len() != 2 {
        return Value::Error(CellError::Value);
    }
    let (x, y) = match (scalar_num(&args[0]), scalar_num(&args[1])) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => return Value::Error(e),
    };
    if x < 0.0 && y.fract() != 0.0 {
        return Value::Error(CellError::Num);
    }
    let r = x.powf(y);
    if r.is_finite() {
        Value::Number(r)
    } else {
        Value::Error(CellError::Num)
    }
}

fn f_andor(args: &[Arg], is_and: bool) -> Value {
    let mut acc = is_and;
    for a in args {
        for v in values_of(a) {
            let t = match v {
                Value::Bool(b) => *b,
                Value::Number(n) => *n != 0.0,
                Value::Empty => continue,
                Value::Text(s) => match s.to_ascii_uppercase().as_str() {
                    "TRUE" => true,
                    "FALSE" => false,
                    _ => continue,
                },
                Value::Error(e) => return Value::Error(*e),
            };
            acc = if is_and { acc && t } else { acc || t };
        }
    }
    Value::Bool(acc)
}

fn f_not(args: &[Arg]) -> Value {
    if args.len() != 1 {
        return Value::Error(CellError::Value);
    }
    match scalar_val(&args[0]).truthy() {
        Ok(t) => Value::Bool(!t),
        Err(e) => Value::Error(e),
    }
}

fn f_len(args: &[Arg]) -> Value {
    if args.len() != 1 {
        return Value::Error(CellError::Value);
    }
    match scalar_val(&args[0]) {
        Value::Error(e) => Value::Error(e),
        v => Value::Number(v.as_text().chars().count() as f64),
    }
}

fn f_text1(args: &[Arg], f: impl Fn(&str) -> String) -> Value {
    if args.len() != 1 {
        return Value::Error(CellError::Value);
    }
    match scalar_val(&args[0]) {
        Value::Error(e) => Value::Error(e),
        v => Value::Text(f(&v.as_text())),
    }
}

fn f_concat(args: &[Arg]) -> Value {
    let mut s = String::new();
    for a in args {
        for v in values_of(a) {
            if let Value::Error(e) = v {
                return Value::Error(*e);
            }
            s.push_str(&v.as_text());
        }
    }
    Value::Text(s)
}

fn f_if(args: &[Expr], ctx: &dyn Context) -> Value {
    if args.len() < 2 || args.len() > 3 {
        return Value::Error(CellError::Value);
    }
    match eval(&args[0], ctx).truthy() {
        Ok(true) => eval(&args[1], ctx),
        Ok(false) => {
            if args.len() == 3 {
                eval(&args[2], ctx)
            } else {
                Value::Bool(false)
            }
        }
        Err(e) => Value::Error(e),
    }
}

fn f_iferror(args: &[Expr], ctx: &dyn Context) -> Value {
    if args.len() != 2 {
        return Value::Error(CellError::Value);
    }
    let v = eval(&args[0], ctx);
    if v.is_error() {
        eval(&args[1], ctx)
    } else {
        v
    }
}
