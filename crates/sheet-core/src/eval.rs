//! Evaluate an `Expr` against a [`Context`] (which knows how to read cells).
//!
//! Evaluation is scalar; functions that work over ranges receive their
//! arguments as [`Arg`]s, where a reference or a range expands to a list of
//! values. Errors propagate the way they do in a real spreadsheet.

use crate::ast::{BinOp, Expr, UnaryOp};
use crate::cellref::CellRef;
use crate::value::{CellError, Value};
use std::cmp::Ordering;

/// How the evaluator reads cells. The recalc engine (M3) and the workbook both
/// implement this; tests use a tiny in-memory grid.
pub trait Context {
    fn cell_value(&self, sheet: Option<&str>, at: CellRef) -> Value;
}

/// A function argument: a single value, or a flattened reference/range.
pub enum Arg {
    Scalar(Value),
    Multi(Vec<Value>),
}

/// Evaluate an expression to a single value.
pub fn eval(expr: &Expr, ctx: &dyn Context) -> Value {
    match expr {
        Expr::Number(n) => Value::Number(*n),
        Expr::Text(s) => Value::Text(s.clone()),
        Expr::Bool(b) => Value::Bool(*b),
        Expr::Error(e) => Value::Error(*e),
        Expr::Name(_) => Value::Error(CellError::Name),
        Expr::Ref(r) => ctx.cell_value(r.sheet.as_deref(), r.part.cell),
        Expr::Range(_) => Value::Error(CellError::Value), // a range used where a scalar is expected
        Expr::Unary(op, e) => eval_unary(*op, eval(e, ctx)),
        Expr::Binary(op, a, b) => eval_binary(*op, eval(a, ctx), eval(b, ctx)),
        Expr::Func(name, args) => crate::functions::call(name, args, ctx),
    }
}

/// Evaluate one function argument: references/ranges become `Multi`, so that
/// `SUM(A1)` ignores text in `A1` (range semantics) while `SUM("5")` coerces.
pub fn eval_arg(expr: &Expr, ctx: &dyn Context) -> Arg {
    match expr {
        Expr::Ref(r) => Arg::Multi(vec![ctx.cell_value(r.sheet.as_deref(), r.part.cell)]),
        Expr::Range(r) => Arg::Multi(
            r.range().cells().map(|c| ctx.cell_value(r.sheet.as_deref(), c)).collect(),
        ),
        other => Arg::Scalar(eval(other, ctx)),
    }
}

fn eval_unary(op: UnaryOp, v: Value) -> Value {
    match v.as_number() {
        Err(e) => Value::Error(e),
        Ok(n) => match op {
            UnaryOp::Neg => Value::Number(-n),
            UnaryOp::Pos => Value::Number(n),
            UnaryOp::Percent => Value::Number(n / 100.0),
        },
    }
}

fn eval_binary(op: BinOp, a: Value, b: Value) -> Value {
    // errors short-circuit
    if let Value::Error(e) = a {
        return Value::Error(e);
    }
    if let Value::Error(e) = b {
        return Value::Error(e);
    }
    match op {
        BinOp::Concat => Value::Text(format!("{}{}", a.as_text(), b.as_text())),
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Pow => {
            let (x, y) = match (a.as_number(), b.as_number()) {
                (Ok(x), Ok(y)) => (x, y),
                (Err(e), _) | (_, Err(e)) => return Value::Error(e),
            };
            match op {
                BinOp::Add => Value::Number(x + y),
                BinOp::Sub => Value::Number(x - y),
                BinOp::Mul => Value::Number(x * y),
                BinOp::Div => {
                    if y == 0.0 {
                        Value::Error(CellError::Div0)
                    } else {
                        Value::Number(x / y)
                    }
                }
                BinOp::Pow => {
                    if x < 0.0 && y.fract() != 0.0 {
                        Value::Error(CellError::Num)
                    } else {
                        let r = x.powf(y);
                        if r.is_finite() {
                            Value::Number(r)
                        } else {
                            Value::Error(CellError::Num)
                        }
                    }
                }
                _ => unreachable!(),
            }
        }
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
            let ord = compare(&a, &b);
            let res = match op {
                BinOp::Eq => ord == Ordering::Equal,
                BinOp::Ne => ord != Ordering::Equal,
                BinOp::Lt => ord == Ordering::Less,
                BinOp::Gt => ord == Ordering::Greater,
                BinOp::Le => ord != Ordering::Greater,
                BinOp::Ge => ord != Ordering::Less,
                _ => unreachable!(),
            };
            Value::Bool(res)
        }
    }
}

/// Excel-ish comparison ordering: by type rank (Number < Text < Bool, with
/// Empty lowest), then by value; text is compared case-insensitively.
fn compare(a: &Value, b: &Value) -> Ordering {
    fn rank(v: &Value) -> u8 {
        match v {
            Value::Empty => 0,
            Value::Number(_) => 1,
            Value::Text(_) => 2,
            Value::Bool(_) => 3,
            Value::Error(_) => 4,
        }
    }
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Text(x), Value::Text(y)) => {
            x.to_lowercase().cmp(&y.to_lowercase())
        }
        (Value::Empty, Value::Empty) => Ordering::Equal,
        _ => rank(a).cmp(&rank(b)),
    }
}
