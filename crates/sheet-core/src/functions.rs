//! The built-in function library. Most functions take already evaluated
//! [`Arg`]s. Two other groups read the *raw* argument expressions instead:
//!
//! * the *lazy* ones (`IF`, `IFERROR`, `IFS`, `IFNA`), so only the chosen branch
//!   is evaluated, exactly as a spreadsheet expects; and
//! * the *geometry* ones (`VLOOKUP`, `HLOOKUP`, `INDEX`, `MATCH`, and the
//!   `…IF` aggregates plus `SUMPRODUCT`), which need a range's rows and columns
//!   — information that is lost once a range is flattened into a value list.

use crate::ast::Expr;
use crate::eval::{eval, eval_arg, Arg, Context};
use crate::value::{CellError, Value};
use std::cmp::Ordering;

pub fn call(name: &str, arg_exprs: &[Expr], ctx: &dyn Context) -> Value {
    // Lazy functions look at the raw expressions (only some branches run).
    match name {
        "IF" => return f_if(arg_exprs, ctx),
        "IFERROR" => return f_iferror(arg_exprs, ctx),
        "IFS" => return f_ifs(arg_exprs, ctx),
        "IFNA" => return f_ifna(arg_exprs, ctx),
        _ => {}
    }
    // Geometry-aware functions need the shape of their range arguments, which is
    // only available from the raw expressions (before flattening to a list).
    match name {
        "VLOOKUP" => return f_vlookup(arg_exprs, ctx),
        "HLOOKUP" => return f_hlookup(arg_exprs, ctx),
        "INDEX" => return f_index(arg_exprs, ctx),
        "MATCH" => return f_match(arg_exprs, ctx),
        "SUMIF" => return f_sumif(arg_exprs, ctx, AggKind::Sum),
        "AVERAGEIF" => return f_sumif(arg_exprs, ctx, AggKind::Average),
        "COUNTIF" => return f_countif(arg_exprs, ctx),
        "SUMPRODUCT" => return f_sumproduct(arg_exprs, ctx),
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

        // ---- math --------------------------------------------------------
        "CEILING" => f_ceiling_floor(&args, true),
        "FLOOR" => f_ceiling_floor(&args, false),
        "ROUNDUP" => f_round_dir(&args, RoundDir::Up),
        "ROUNDDOWN" => f_round_dir(&args, RoundDir::Down),
        "TRUNC" => f_round_dir(&args, RoundDir::Trunc),
        "SIGN" => map1(&args, |x| Ok(if x > 0.0 { 1.0 } else if x < 0.0 { -1.0 } else { 0.0 })),
        "EXP" => map1(&args, |x| finite(x.exp())),
        "LN" => map1(&args, |x| if x <= 0.0 { Err(CellError::Num) } else { Ok(x.ln()) }),
        "LOG" => f_log(&args),
        "LOG10" => map1(&args, |x| if x <= 0.0 { Err(CellError::Num) } else { Ok(x.log10()) }),
        "PI" => f_nullary(&args, std::f64::consts::PI),
        "RAND" => f_rand(&args),
        "RANDBETWEEN" => f_randbetween(&args),

        // ---- stats -------------------------------------------------------
        "MEDIAN" => f_median(&args),
        "STDEV" => f_var_stdev(&args, true),
        "VAR" => f_var_stdev(&args, false),
        "LARGE" => f_large_small(&args, true),
        "SMALL" => f_large_small(&args, false),

        // ---- text --------------------------------------------------------
        "LEFT" => f_left_right(&args, true),
        "RIGHT" => f_left_right(&args, false),
        "MID" => f_mid(&args),
        "FIND" => f_find(&args, true),
        "SEARCH" => f_find(&args, false),
        "SUBSTITUTE" => f_substitute(&args),
        "REPT" => f_rept(&args),
        "PROPER" => f_text1(&args, |s| proper_case(s)),
        "EXACT" => f_exact(&args),
        "TEXTJOIN" => f_textjoin(&args),

        // ---- logical / info ---------------------------------------------
        "XOR" => f_xor(&args),
        "ISBLANK" => f_is(&args, |v| matches!(v, Value::Empty)),
        "ISNUMBER" => f_is(&args, |v| matches!(v, Value::Number(_))),
        "ISTEXT" => f_is(&args, |v| matches!(v, Value::Text(_))),
        "ISLOGICAL" => f_is(&args, |v| matches!(v, Value::Bool(_))),
        "ISERROR" => f_is(&args, |v| matches!(v, Value::Error(_))),
        "ISERR" => f_is(&args, |v| matches!(v, Value::Error(e) if *e != CellError::NA)),
        "ISNA" => f_is(&args, |v| matches!(v, Value::Error(CellError::NA))),
        "NA" => Value::Error(CellError::NA),

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

/// Coerce a possibly-infinite/NaN result into a number or `#NUM!`.
fn finite(x: f64) -> Result<f64, CellError> {
    if x.is_finite() {
        Ok(x)
    } else {
        Err(CellError::Num)
    }
}

/// A fast, dependency-free pseudo-random `f64` in `[0, 1)`. Seeded from the
/// clock so successive calls differ; this is plenty for spreadsheet `RAND`.
fn rand_unit() -> f64 {
    use std::cell::Cell;
    use std::time::{SystemTime, UNIX_EPOCH};
    thread_local! {
        static STATE: Cell<u64> = Cell::new(0);
    }
    STATE.with(|s| {
        let mut x = s.get();
        if x == 0 {
            x = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x9E3779B97F4A7C15)
                | 1;
        }
        // xorshift64*
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        s.set(x);
        let v = x.wrapping_mul(0x2545F4914F6CDD1D);
        // top 53 bits -> [0, 1)
        (v >> 11) as f64 / (1u64 << 53) as f64
    })
}

// ---- comparison / criteria -------------------------------------------------

/// Excel-ish ordering used by lookups and criteria: numbers below text below
/// booleans (empty lowest); text compared case-insensitively.
fn cmp_values(a: &Value, b: &Value) -> Ordering {
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
        (Value::Text(x), Value::Text(y)) => x.to_lowercase().cmp(&y.to_lowercase()),
        (Value::Empty, Value::Empty) => Ordering::Equal,
        _ => rank(a).cmp(&rank(b)),
    }
}

/// A parsed `SUMIF`/`COUNTIF` criterion: an optional comparison operator plus a
/// target value. A bare value means "equals".
struct Criterion {
    op: CritOp,
    target: Value,
}

#[derive(Clone, Copy, PartialEq)]
enum CritOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl Criterion {
    /// Build from a criterion value, parsing a leading operator out of text such
    /// as `">=3"` or `"<>foo"`. Numbers/bools become an equality test directly.
    fn parse(v: &Value) -> Criterion {
        let s = match v {
            Value::Text(s) => s.clone(),
            other => {
                return Criterion { op: CritOp::Eq, target: other.clone() };
            }
        };
        let (op, rest) = if let Some(r) = s.strip_prefix(">=") {
            (CritOp::Ge, r)
        } else if let Some(r) = s.strip_prefix("<=") {
            (CritOp::Le, r)
        } else if let Some(r) = s.strip_prefix("<>") {
            (CritOp::Ne, r)
        } else if let Some(r) = s.strip_prefix('>') {
            (CritOp::Gt, r)
        } else if let Some(r) = s.strip_prefix('<') {
            (CritOp::Lt, r)
        } else if let Some(r) = s.strip_prefix('=') {
            (CritOp::Eq, r)
        } else {
            (CritOp::Eq, s.as_str())
        };
        Criterion { op, target: Value::parse_literal(rest) }
    }

    /// Does `cell` satisfy this criterion? Equality on text is case-insensitive
    /// (matching the sheet's comparison rules).
    fn matches(&self, cell: &Value) -> bool {
        let ord = cmp_values(cell, &self.target);
        match self.op {
            CritOp::Eq => ord == Ordering::Equal,
            CritOp::Ne => ord != Ordering::Equal,
            CritOp::Lt => ord == Ordering::Less,
            CritOp::Le => ord != Ordering::Greater,
            CritOp::Gt => ord == Ordering::Greater,
            CritOp::Ge => ord != Ordering::Less,
        }
    }
}

// ---- raw-range helpers (geometry-aware functions) --------------------------

/// Read an argument that is expected to be a range into a 2-D grid of values
/// (row-major), preserving its shape. A lone cell reference becomes 1x1.
fn read_grid(expr: &Expr, ctx: &dyn Context) -> Option<(usize, usize, Vec<Value>)> {
    match expr {
        Expr::Range(rr) => {
            let r = rr.range();
            let (w, h) = (r.width() as usize, r.height() as usize);
            let cells = r.cells().map(|c| ctx.cell_value(rr.sheet.as_deref(), c)).collect();
            Some((w, h, cells))
        }
        Expr::Ref(re) => Some((1, 1, vec![ctx.cell_value(re.sheet.as_deref(), re.part.cell)])),
        _ => None,
    }
}

// ---- math ------------------------------------------------------------------

fn f_nullary(args: &[Arg], v: f64) -> Value {
    if args.is_empty() {
        Value::Number(v)
    } else {
        Value::Error(CellError::Value)
    }
}

fn f_ceiling_floor(args: &[Arg], ceil: bool) -> Value {
    if args.len() != 2 {
        return Value::Error(CellError::Value);
    }
    let (x, sig) = match (scalar_num(&args[0]), scalar_num(&args[1])) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => return Value::Error(e),
    };
    if sig == 0.0 {
        // CEILING/FLOOR to a multiple of zero is zero in Excel.
        return Value::Number(0.0);
    }
    // Excel errors if the value and significance have opposite signs.
    if (x > 0.0 && sig < 0.0) || (x < 0.0 && sig > 0.0) {
        return Value::Error(CellError::Num);
    }
    let q = x / sig;
    let n = if ceil { q.ceil() } else { q.floor() };
    Value::Number(n * sig)
}

#[derive(Clone, Copy)]
enum RoundDir {
    Up,
    Down,
    Trunc,
}

fn f_round_dir(args: &[Arg], dir: RoundDir) -> Value {
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
    let scaled = x * f;
    let r = match dir {
        // ROUNDUP rounds away from zero; ROUNDDOWN and TRUNC toward zero.
        RoundDir::Up => {
            if scaled >= 0.0 {
                scaled.ceil()
            } else {
                scaled.floor()
            }
        }
        RoundDir::Down | RoundDir::Trunc => scaled.trunc(),
    };
    Value::Number(r / f)
}

fn f_log(args: &[Arg]) -> Value {
    if args.is_empty() || args.len() > 2 {
        return Value::Error(CellError::Value);
    }
    let x = match scalar_num(&args[0]) {
        Ok(v) => v,
        Err(e) => return Value::Error(e),
    };
    let base = if args.len() == 2 {
        match scalar_num(&args[1]) {
            Ok(v) => v,
            Err(e) => return Value::Error(e),
        }
    } else {
        10.0
    };
    if x <= 0.0 || base <= 0.0 || base == 1.0 {
        return Value::Error(CellError::Num);
    }
    Value::Number(x.log(base))
}

fn f_rand(args: &[Arg]) -> Value {
    if !args.is_empty() {
        return Value::Error(CellError::Value);
    }
    Value::Number(rand_unit())
}

fn f_randbetween(args: &[Arg]) -> Value {
    if args.len() != 2 {
        return Value::Error(CellError::Value);
    }
    let (lo, hi) = match (scalar_num(&args[0]), scalar_num(&args[1])) {
        (Ok(a), Ok(b)) => (a.ceil() as i64, b.floor() as i64),
        (Err(e), _) | (_, Err(e)) => return Value::Error(e),
    };
    if lo > hi {
        return Value::Error(CellError::Num);
    }
    let span = (hi - lo + 1) as f64;
    let n = lo + (rand_unit() * span) as i64;
    Value::Number(n.min(hi) as f64)
}

// ---- stats -----------------------------------------------------------------

fn f_median(args: &[Arg]) -> Value {
    let mut ns = match numbers(args) {
        Ok(ns) => ns,
        Err(e) => return Value::Error(e),
    };
    if ns.is_empty() {
        return Value::Error(CellError::Num);
    }
    ns.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let mid = ns.len() / 2;
    let m = if ns.len() % 2 == 1 {
        ns[mid]
    } else {
        (ns[mid - 1] + ns[mid]) / 2.0
    };
    Value::Number(m)
}

fn f_var_stdev(args: &[Arg], stdev: bool) -> Value {
    let ns = match numbers(args) {
        Ok(ns) => ns,
        Err(e) => return Value::Error(e),
    };
    if ns.len() < 2 {
        // Sample variance/stdev needs at least two points.
        return Value::Error(CellError::Div0);
    }
    let n = ns.len() as f64;
    let mean = ns.iter().sum::<f64>() / n;
    let ss = ns.iter().map(|x| (x - mean).powi(2)).sum::<f64>();
    let var = ss / (n - 1.0);
    Value::Number(if stdev { var.sqrt() } else { var })
}

fn f_large_small(args: &[Arg], large: bool) -> Value {
    if args.len() != 2 {
        return Value::Error(CellError::Value);
    }
    let k = match scalar_num(&args[1]) {
        Ok(v) => v as i64,
        Err(e) => return Value::Error(e),
    };
    let mut ns = match numbers(&args[..1]) {
        Ok(ns) => ns,
        Err(e) => return Value::Error(e),
    };
    if k < 1 || k as usize > ns.len() {
        return Value::Error(CellError::Num);
    }
    ns.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let idx = if large { ns.len() - k as usize } else { k as usize - 1 };
    Value::Number(ns[idx])
}

// ---- text ------------------------------------------------------------------

fn proper_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut start_word = true;
    for ch in s.chars() {
        if ch.is_alphabetic() {
            if start_word {
                out.extend(ch.to_uppercase());
            } else {
                out.extend(ch.to_lowercase());
            }
            start_word = false;
        } else {
            out.push(ch);
            start_word = true;
        }
    }
    out
}

fn f_left_right(args: &[Arg], left: bool) -> Value {
    if args.is_empty() || args.len() > 2 {
        return Value::Error(CellError::Value);
    }
    let s = match scalar_val(&args[0]) {
        Value::Error(e) => return Value::Error(e),
        v => v.as_text(),
    };
    let n = if args.len() == 2 {
        match scalar_num(&args[1]) {
            Ok(v) if v < 0.0 => return Value::Error(CellError::Value),
            Ok(v) => v as usize,
            Err(e) => return Value::Error(e),
        }
    } else {
        1
    };
    let chars: Vec<char> = s.chars().collect();
    let take = n.min(chars.len());
    let slice: String = if left {
        chars[..take].iter().collect()
    } else {
        chars[chars.len() - take..].iter().collect()
    };
    Value::Text(slice)
}

fn f_mid(args: &[Arg]) -> Value {
    if args.len() != 3 {
        return Value::Error(CellError::Value);
    }
    let s = match scalar_val(&args[0]) {
        Value::Error(e) => return Value::Error(e),
        v => v.as_text(),
    };
    let start = match scalar_num(&args[1]) {
        Ok(v) if v < 1.0 => return Value::Error(CellError::Value),
        Ok(v) => v as usize,
        Err(e) => return Value::Error(e),
    };
    let count = match scalar_num(&args[2]) {
        Ok(v) if v < 0.0 => return Value::Error(CellError::Value),
        Ok(v) => v as usize,
        Err(e) => return Value::Error(e),
    };
    let chars: Vec<char> = s.chars().collect();
    if start > chars.len() {
        return Value::Text(String::new());
    }
    let from = start - 1;
    let to = (from + count).min(chars.len());
    Value::Text(chars[from..to].iter().collect())
}

/// `FIND`/`SEARCH`: 1-based position of `needle` in `haystack`, optionally from
/// a start position. `FIND` is case-sensitive; `SEARCH` is not. Missing → `#VALUE!`.
fn f_find(args: &[Arg], case_sensitive: bool) -> Value {
    if args.len() < 2 || args.len() > 3 {
        return Value::Error(CellError::Value);
    }
    let needle = match scalar_val(&args[0]) {
        Value::Error(e) => return Value::Error(e),
        v => v.as_text(),
    };
    let hay = match scalar_val(&args[1]) {
        Value::Error(e) => return Value::Error(e),
        v => v.as_text(),
    };
    let start = if args.len() == 3 {
        match scalar_num(&args[2]) {
            Ok(v) if v < 1.0 => return Value::Error(CellError::Value),
            Ok(v) => v as usize,
            Err(e) => return Value::Error(e),
        }
    } else {
        1
    };
    let (hay_cmp, needle_cmp) = if case_sensitive {
        (hay.clone(), needle.clone())
    } else {
        (hay.to_lowercase(), needle.to_lowercase())
    };
    let hay_chars: Vec<char> = hay_cmp.chars().collect();
    if start > hay_chars.len() + 1 {
        return Value::Error(CellError::Value);
    }
    let needle_chars: Vec<char> = needle_cmp.chars().collect();
    if needle_chars.is_empty() {
        return Value::Number(start as f64);
    }
    // Search character-by-character so positions are codepoint-based.
    let begin = start - 1;
    if begin + needle_chars.len() <= hay_chars.len() {
        for i in begin..=hay_chars.len() - needle_chars.len() {
            if hay_chars[i..i + needle_chars.len()] == needle_chars[..] {
                return Value::Number((i + 1) as f64);
            }
        }
    }
    Value::Error(CellError::Value)
}

fn f_substitute(args: &[Arg]) -> Value {
    if args.len() < 3 || args.len() > 4 {
        return Value::Error(CellError::Value);
    }
    let text = match scalar_val(&args[0]) {
        Value::Error(e) => return Value::Error(e),
        v => v.as_text(),
    };
    let old = match scalar_val(&args[1]) {
        Value::Error(e) => return Value::Error(e),
        v => v.as_text(),
    };
    let new = match scalar_val(&args[2]) {
        Value::Error(e) => return Value::Error(e),
        v => v.as_text(),
    };
    if old.is_empty() {
        return Value::Text(text);
    }
    if args.len() == 4 {
        // Replace only the nth occurrence (1-based).
        let nth = match scalar_num(&args[3]) {
            Ok(v) if v < 1.0 => return Value::Error(CellError::Value),
            Ok(v) => v as usize,
            Err(e) => return Value::Error(e),
        };
        let mut result = String::new();
        let mut rest = text.as_str();
        let mut count = 0usize;
        while let Some(pos) = rest.find(&old) {
            count += 1;
            if count == nth {
                result.push_str(&rest[..pos]);
                result.push_str(&new);
                result.push_str(&rest[pos + old.len()..]);
                return Value::Text(result);
            }
            result.push_str(&rest[..pos + old.len()]);
            rest = &rest[pos + old.len()..];
        }
        result.push_str(rest);
        Value::Text(result)
    } else {
        Value::Text(text.replace(&old, &new))
    }
}

fn f_rept(args: &[Arg]) -> Value {
    if args.len() != 2 {
        return Value::Error(CellError::Value);
    }
    let s = match scalar_val(&args[0]) {
        Value::Error(e) => return Value::Error(e),
        v => v.as_text(),
    };
    let n = match scalar_num(&args[1]) {
        Ok(v) if v < 0.0 => return Value::Error(CellError::Value),
        Ok(v) => v as usize,
        Err(e) => return Value::Error(e),
    };
    Value::Text(s.repeat(n))
}

fn f_exact(args: &[Arg]) -> Value {
    if args.len() != 2 {
        return Value::Error(CellError::Value);
    }
    let a = match scalar_val(&args[0]) {
        Value::Error(e) => return Value::Error(e),
        v => v.as_text(),
    };
    let b = match scalar_val(&args[1]) {
        Value::Error(e) => return Value::Error(e),
        v => v.as_text(),
    };
    Value::Bool(a == b)
}

/// `TEXTJOIN(delimiter, ignore_empty, text1, …)`: join values with a separator,
/// optionally skipping blanks. Ranges expand to all their cells.
fn f_textjoin(args: &[Arg]) -> Value {
    if args.len() < 3 {
        return Value::Error(CellError::Value);
    }
    let delim = match scalar_val(&args[0]) {
        Value::Error(e) => return Value::Error(e),
        v => v.as_text(),
    };
    let ignore_empty = match scalar_val(&args[1]).truthy() {
        Ok(b) => b,
        Err(e) => return Value::Error(e),
    };
    let mut parts: Vec<String> = Vec::new();
    for a in &args[2..] {
        for v in values_of(a) {
            if let Value::Error(e) = v {
                return Value::Error(*e);
            }
            if ignore_empty && matches!(v, Value::Empty) {
                continue;
            }
            parts.push(v.as_text());
        }
    }
    Value::Text(parts.join(&delim))
}

// ---- logical / info --------------------------------------------------------

fn f_xor(args: &[Arg]) -> Value {
    let mut acc = false;
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
            acc ^= t;
        }
    }
    Value::Bool(acc)
}

fn f_is(args: &[Arg], pred: impl Fn(&Value) -> bool) -> Value {
    if args.len() != 1 {
        return Value::Error(CellError::Value);
    }
    // IS* functions inspect the raw value and never propagate errors themselves.
    Value::Bool(pred(&scalar_val(&args[0])))
}

// ---- lazy logical ----------------------------------------------------------

fn f_ifs(args: &[Expr], ctx: &dyn Context) -> Value {
    if args.is_empty() || args.len() % 2 != 0 {
        return Value::Error(CellError::Value);
    }
    for pair in args.chunks(2) {
        match eval(&pair[0], ctx).truthy() {
            Ok(true) => return eval(&pair[1], ctx),
            Ok(false) => continue,
            Err(e) => return Value::Error(e),
        }
    }
    // No condition matched.
    Value::Error(CellError::NA)
}

fn f_ifna(args: &[Expr], ctx: &dyn Context) -> Value {
    if args.len() != 2 {
        return Value::Error(CellError::Value);
    }
    let v = eval(&args[0], ctx);
    if matches!(v, Value::Error(CellError::NA)) {
        eval(&args[1], ctx)
    } else {
        v
    }
}

// ---- lookup (geometry-aware) -----------------------------------------------

fn f_vlookup(args: &[Expr], ctx: &dyn Context) -> Value {
    f_vhlookup(args, ctx, true)
}

fn f_hlookup(args: &[Expr], ctx: &dyn Context) -> Value {
    f_vhlookup(args, ctx, false)
}

/// `VLOOKUP(key, table, index, [range_lookup])` / `HLOOKUP(...)`. We do an exact
/// scan of the first column (V) or row (H); the optional `range_lookup` flag is
/// accepted but exact matching is used either way for v1.
fn f_vhlookup(args: &[Expr], ctx: &dyn Context, vertical: bool) -> Value {
    if args.len() < 3 || args.len() > 4 {
        return Value::Error(CellError::Value);
    }
    let key = eval(&args[0], ctx);
    if let Value::Error(e) = key {
        return Value::Error(e);
    }
    let (w, h, cells) = match read_grid(&args[1], ctx) {
        Some(g) => g,
        None => return Value::Error(CellError::Value),
    };
    let idx = match eval(&args[2], ctx).as_number() {
        Ok(v) => v as i64,
        Err(e) => return Value::Error(e),
    };
    let at = |c: usize, r: usize| cells[r * w + c].clone();
    if vertical {
        if idx < 1 || idx as usize > w {
            return Value::Error(CellError::Ref);
        }
        for r in 0..h {
            if cmp_values(&at(0, r), &key) == Ordering::Equal {
                return at(idx as usize - 1, r);
            }
        }
    } else {
        if idx < 1 || idx as usize > h {
            return Value::Error(CellError::Ref);
        }
        for c in 0..w {
            if cmp_values(&at(c, 0), &key) == Ordering::Equal {
                return at(c, idx as usize - 1);
            }
        }
    }
    Value::Error(CellError::NA)
}

/// `INDEX(range, row, [col])`: the value at 1-based `row`/`col` of a range. With
/// a single-row or single-column range the lone index selects along that axis.
fn f_index(args: &[Expr], ctx: &dyn Context) -> Value {
    if args.len() < 2 || args.len() > 3 {
        return Value::Error(CellError::Value);
    }
    let (w, h, cells) = match read_grid(&args[0], ctx) {
        Some(g) => g,
        None => return Value::Error(CellError::Value),
    };
    let row = match eval(&args[1], ctx).as_number() {
        Ok(v) => v as i64,
        Err(e) => return Value::Error(e),
    };
    let col = if args.len() == 3 {
        match eval(&args[2], ctx).as_number() {
            Ok(v) => v as i64,
            Err(e) => return Value::Error(e),
        }
    } else {
        0
    };
    // With one index over a single-row range, it selects the column; otherwise
    // (single-column or 2-D) the lone index is the row.
    let (r, c) = if args.len() == 2 {
        if h == 1 {
            (1, row)
        } else {
            (row, 1)
        }
    } else {
        (row, col)
    };
    if r < 1 || r as usize > h || c < 1 || c as usize > w {
        return Value::Error(CellError::Ref);
    }
    cells[(r as usize - 1) * w + (c as usize - 1)].clone()
}

/// `MATCH(key, range, [match_type])`: 1-based position of `key` within a 1-D
/// range. We implement exact match (type 0); any other type also matches exactly.
fn f_match(args: &[Expr], ctx: &dyn Context) -> Value {
    if args.len() < 2 || args.len() > 3 {
        return Value::Error(CellError::Value);
    }
    let key = eval(&args[0], ctx);
    if let Value::Error(e) = key {
        return Value::Error(e);
    }
    let (_w, _h, cells) = match read_grid(&args[1], ctx) {
        Some(g) => g,
        None => return Value::Error(CellError::Value),
    };
    for (i, v) in cells.iter().enumerate() {
        if cmp_values(v, &key) == Ordering::Equal {
            return Value::Number((i + 1) as f64);
        }
    }
    Value::Error(CellError::NA)
}

// ---- conditional aggregates (geometry-aware) -------------------------------

#[derive(Clone, Copy)]
enum AggKind {
    Sum,
    Average,
}

/// `SUMIF(range, criteria, [sum_range])` and `AVERAGEIF(...)`. Cells in `range`
/// are tested against `criteria`; the matching positions are summed/averaged
/// from `sum_range` if given, otherwise from `range` itself.
fn f_sumif(args: &[Expr], ctx: &dyn Context, kind: AggKind) -> Value {
    if args.len() < 2 || args.len() > 3 {
        return Value::Error(CellError::Value);
    }
    let (_cw, _ch, crange) = match read_grid(&args[0], ctx) {
        Some(g) => g,
        None => return Value::Error(CellError::Value),
    };
    let crit = Criterion::parse(&eval(&args[1], ctx));
    let sum_cells = if args.len() == 3 {
        match read_grid(&args[2], ctx) {
            Some((_, _, c)) => c,
            None => return Value::Error(CellError::Value),
        }
    } else {
        crange.clone()
    };
    let mut total = 0.0;
    let mut count = 0u64;
    for (i, cell) in crange.iter().enumerate() {
        if crit.matches(cell) {
            if let Some(target) = sum_cells.get(i) {
                match target {
                    Value::Number(n) => {
                        total += *n;
                        count += 1;
                    }
                    Value::Error(e) => return Value::Error(*e),
                    _ => {
                        // Non-numeric matches still count toward AVERAGEIF? No —
                        // only numeric cells contribute, like Excel.
                    }
                }
            }
        }
    }
    match kind {
        AggKind::Sum => Value::Number(total),
        AggKind::Average => {
            if count == 0 {
                Value::Error(CellError::Div0)
            } else {
                Value::Number(total / count as f64)
            }
        }
    }
}

/// `COUNTIF(range, criteria)`: how many cells in `range` satisfy `criteria`.
fn f_countif(args: &[Expr], ctx: &dyn Context) -> Value {
    if args.len() != 2 {
        return Value::Error(CellError::Value);
    }
    let (_w, _h, cells) = match read_grid(&args[0], ctx) {
        Some(g) => g,
        None => return Value::Error(CellError::Value),
    };
    let crit = Criterion::parse(&eval(&args[1], ctx));
    let n = cells.iter().filter(|c| crit.matches(c)).count();
    Value::Number(n as f64)
}

/// `SUMPRODUCT(a, b, …)`: multiply parallel arrays element-wise, then sum. All
/// ranges must share the same number of cells. Non-numeric cells count as zero.
fn f_sumproduct(args: &[Expr], ctx: &dyn Context) -> Value {
    if args.is_empty() {
        return Value::Error(CellError::Value);
    }
    let mut grids: Vec<Vec<Value>> = Vec::with_capacity(args.len());
    for e in args {
        match read_grid(e, ctx) {
            Some((_, _, cells)) => grids.push(cells),
            None => {
                // A scalar argument acts as a 1-element array.
                let v = eval(e, ctx);
                if let Value::Error(err) = v {
                    return Value::Error(err);
                }
                grids.push(vec![v]);
            }
        }
    }
    let len = grids[0].len();
    if grids.iter().any(|g| g.len() != len) {
        return Value::Error(CellError::Value);
    }
    let mut total = 0.0;
    for i in 0..len {
        let mut prod = 1.0;
        for g in &grids {
            match &g[i] {
                Value::Number(n) => prod *= n,
                Value::Bool(b) => prod *= if *b { 1.0 } else { 0.0 },
                Value::Error(e) => return Value::Error(*e),
                // Text/empty contribute zero to the product (entire term drops).
                _ => {
                    prod = 0.0;
                }
            }
        }
        total += prod;
    }
    Value::Number(total)
}

// ---- original lazy functions ----------------------------------------------

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{RangeRef, RefPart, Reference};
    use crate::cellref::CellRef;
    use std::collections::HashMap;

    /// A tiny in-memory grid for exercising the function library directly.
    struct Grid(HashMap<CellRef, Value>);

    impl Context for Grid {
        fn cell_value(&self, _sheet: Option<&str>, at: CellRef) -> Value {
            self.0.get(&at).cloned().unwrap_or(Value::Empty)
        }
    }

    fn grid(cells: &[(&str, Value)]) -> Grid {
        let mut m = HashMap::new();
        for (a1, v) in cells {
            m.insert(CellRef::parse(a1).unwrap(), v.clone());
        }
        Grid(m)
    }

    fn part(a1: &str) -> RefPart {
        RefPart { cell: CellRef::parse(a1).unwrap(), abs_col: false, abs_row: false }
    }

    /// Build a cell reference expression, e.g. `cref("A1")`.
    fn cref(a1: &str) -> Expr {
        Expr::Ref(Reference { sheet: None, part: part(a1) })
    }

    /// Build a range expression, e.g. `rng("A1", "B3")`.
    fn rng(a: &str, b: &str) -> Expr {
        Expr::Range(RangeRef { sheet: None, a: part(a), b: part(b) })
    }

    fn num(x: f64) -> Expr {
        Expr::Number(x)
    }

    fn text(s: &str) -> Expr {
        Expr::Text(s.into())
    }

    fn run(name: &str, args: &[Expr], ctx: &dyn Context) -> Value {
        call(name, args, ctx)
    }

    // Convenience constructors for expected values.
    fn n(x: f64) -> Value {
        Value::Number(x)
    }
    fn t(s: &str) -> Value {
        Value::Text(s.into())
    }

    #[test]
    fn existing_functions_still_work() {
        let g = grid(&[("A1", n(1.0)), ("A2", n(2.0)), ("A3", n(3.0))]);
        assert_eq!(run("SUM", &[rng("A1", "A3")], &g), n(6.0));
        assert_eq!(run("AVERAGE", &[rng("A1", "A3")], &g), n(2.0));
        assert_eq!(run("UPPER", &[text("hi")], &g), t("HI"));
    }

    #[test]
    fn vlookup_exact() {
        // A column of keys, B column of values.
        let g = grid(&[
            ("A1", n(10.0)),
            ("B1", t("ten")),
            ("A2", n(20.0)),
            ("B2", t("twenty")),
            ("A3", n(30.0)),
            ("B3", t("thirty")),
        ]);
        assert_eq!(run("VLOOKUP", &[num(20.0), rng("A1", "B3"), num(2.0)], &g), t("twenty"));
        // Missing key -> #N/A.
        assert_eq!(
            run("VLOOKUP", &[num(99.0), rng("A1", "B3"), num(2.0)], &g),
            Value::Error(CellError::NA)
        );
        // Column index out of range -> #REF!.
        assert_eq!(
            run("VLOOKUP", &[num(20.0), rng("A1", "B3"), num(5.0)], &g),
            Value::Error(CellError::Ref)
        );
    }

    #[test]
    fn hlookup_exact() {
        // A row of keys, the next row of values.
        let g = grid(&[
            ("A1", t("x")),
            ("B1", t("y")),
            ("C1", t("z")),
            ("A2", n(1.0)),
            ("B2", n(2.0)),
            ("C2", n(3.0)),
        ]);
        assert_eq!(run("HLOOKUP", &[text("y"), rng("A1", "C2"), num(2.0)], &g), n(2.0));
    }

    #[test]
    fn index_and_match() {
        let g = grid(&[("A1", n(5.0)), ("B1", n(6.0)), ("A2", n(7.0)), ("B2", n(8.0))]);
        // INDEX(range, row, col)
        assert_eq!(run("INDEX", &[rng("A1", "B2"), num(2.0), num(1.0)], &g), n(7.0));
        // INDEX over a single column with one index.
        assert_eq!(run("INDEX", &[rng("A1", "A2"), num(2.0)], &g), n(7.0));
        // Out-of-bounds -> #REF!.
        assert_eq!(
            run("INDEX", &[rng("A1", "B2"), num(3.0), num(1.0)], &g),
            Value::Error(CellError::Ref)
        );

        let g2 = grid(&[("A1", t("a")), ("A2", t("b")), ("A3", t("c"))]);
        // MATCH exact.
        assert_eq!(run("MATCH", &[text("b"), rng("A1", "A3"), num(0.0)], &g2), n(2.0));
        assert_eq!(
            run("MATCH", &[text("z"), rng("A1", "A3"), num(0.0)], &g2),
            Value::Error(CellError::NA)
        );
    }

    #[test]
    fn sumif_countif_averageif() {
        let g = grid(&[("A1", n(1.0)), ("A2", n(5.0)), ("A3", n(10.0)), ("A4", n(20.0))]);
        // SUMIF with a comparison criterion.
        assert_eq!(run("SUMIF", &[rng("A1", "A4"), text(">=5")], &g), n(35.0));
        // COUNTIF.
        assert_eq!(run("COUNTIF", &[rng("A1", "A4"), text(">5")], &g), n(2.0));
        // AVERAGEIF.
        assert_eq!(run("AVERAGEIF", &[rng("A1", "A4"), text(">=10")], &g), n(15.0));

        // SUMIF with a separate sum_range.
        let g2 = grid(&[
            ("A1", t("x")),
            ("A2", t("y")),
            ("A3", t("x")),
            ("B1", n(1.0)),
            ("B2", n(2.0)),
            ("B3", n(4.0)),
        ]);
        assert_eq!(run("SUMIF", &[rng("A1", "A3"), text("x"), rng("B1", "B3")], &g2), n(5.0));
    }

    #[test]
    fn sumproduct_basic() {
        let g = grid(&[
            ("A1", n(1.0)),
            ("A2", n(2.0)),
            ("A3", n(3.0)),
            ("B1", n(4.0)),
            ("B2", n(5.0)),
            ("B3", n(6.0)),
        ]);
        // 1*4 + 2*5 + 3*6 = 32.
        assert_eq!(run("SUMPRODUCT", &[rng("A1", "A3"), rng("B1", "B3")], &g), n(32.0));
    }

    #[test]
    fn text_left_mid_right() {
        let g = grid(&[]);
        assert_eq!(run("LEFT", &[text("hello"), num(2.0)], &g), t("he"));
        assert_eq!(run("RIGHT", &[text("hello"), num(3.0)], &g), t("llo"));
        assert_eq!(run("MID", &[text("hello"), num(2.0), num(3.0)], &g), t("ell"));
        assert_eq!(run("PROPER", &[text("hello world")], &g), t("Hello World"));
        assert_eq!(run("REPT", &[text("ab"), num(3.0)], &g), t("ababab"));
        assert_eq!(run("FIND", &[text("l"), text("hello")], &g), n(3.0));
        assert_eq!(run("SEARCH", &[text("L"), text("hello")], &g), n(3.0));
        assert_eq!(run("SUBSTITUTE", &[text("a-b-c"), text("-"), text("+")], &g), t("a+b+c"));
        assert_eq!(
            run("TEXTJOIN", &[text(","), Expr::Bool(true), text("a"), text("b")], &g),
            t("a,b")
        );
        assert_eq!(run("EXACT", &[text("Ab"), text("ab")], &g), Value::Bool(false));
    }

    #[test]
    fn stats_median_and_friends() {
        let g = grid(&[("A1", n(3.0)), ("A2", n(1.0)), ("A3", n(2.0)), ("A4", n(4.0))]);
        // Median of {1,2,3,4} = 2.5.
        assert_eq!(run("MEDIAN", &[rng("A1", "A4")], &g), n(2.5));
        // LARGE/SMALL.
        assert_eq!(run("LARGE", &[rng("A1", "A4"), num(1.0)], &g), n(4.0));
        assert_eq!(run("SMALL", &[rng("A1", "A4"), num(2.0)], &g), n(2.0));
        // Sample variance of {1,2,3,4} = 5/3.
        match run("VAR", &[rng("A1", "A4")], &g) {
            Value::Number(v) => assert!((v - 5.0 / 3.0).abs() < 1e-9),
            other => panic!("expected number, got {:?}", other),
        }
    }

    #[test]
    fn math_ceiling_floor_round() {
        let g = grid(&[]);
        assert_eq!(run("CEILING", &[num(2.1), num(1.0)], &g), n(3.0));
        assert_eq!(run("FLOOR", &[num(2.9), num(1.0)], &g), n(2.0));
        assert_eq!(run("ROUNDUP", &[num(2.1), num(0.0)], &g), n(3.0));
        assert_eq!(run("ROUNDDOWN", &[num(2.9), num(0.0)], &g), n(2.0));
        assert_eq!(run("TRUNC", &[num(2.9), num(0.0)], &g), n(2.0));
        assert_eq!(run("SIGN", &[num(-5.0)], &g), n(-1.0));
        assert_eq!(run("PI", &[], &g), n(std::f64::consts::PI));
        // LOG base 2 of 8 = 3.
        assert_eq!(run("LOG", &[num(8.0), num(2.0)], &g), n(3.0));
    }

    #[test]
    fn info_predicates() {
        let g = grid(&[("A1", n(5.0)), ("A2", Value::Empty), ("A3", t("hi"))]);
        assert_eq!(run("ISNUMBER", &[cref("A1")], &g), Value::Bool(true));
        assert_eq!(run("ISNUMBER", &[cref("A3")], &g), Value::Bool(false));
        assert_eq!(run("ISBLANK", &[cref("A2")], &g), Value::Bool(true));
        assert_eq!(run("ISTEXT", &[cref("A3")], &g), Value::Bool(true));
        assert_eq!(run("ISERROR", &[Expr::Error(CellError::Div0)], &g), Value::Bool(true));
        assert_eq!(run("NA", &[], &g), Value::Error(CellError::NA));
        assert_eq!(run("XOR", &[Expr::Bool(true), Expr::Bool(false)], &g), Value::Bool(true));
    }

    #[test]
    fn logical_ifs_and_ifna() {
        let g = grid(&[]);
        // IFS picks the first true branch.
        assert_eq!(
            run("IFS", &[Expr::Bool(false), num(1.0), Expr::Bool(true), num(2.0)], &g),
            n(2.0)
        );
        // IFS with no match -> #N/A.
        assert_eq!(run("IFS", &[Expr::Bool(false), num(1.0)], &g), Value::Error(CellError::NA));
        // IFNA substitutes only on #N/A.
        assert_eq!(run("IFNA", &[Expr::Error(CellError::NA), num(7.0)], &g), n(7.0));
        assert_eq!(
            run("IFNA", &[Expr::Error(CellError::Div0), num(7.0)], &g),
            Value::Error(CellError::Div0)
        );
    }

    #[test]
    fn unknown_function_is_name_error() {
        let g = grid(&[]);
        assert_eq!(run("BOGUS", &[num(1.0)], &g), Value::Error(CellError::Name));
    }
}
