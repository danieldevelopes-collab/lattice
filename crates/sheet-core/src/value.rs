//! What a cell *is* once computed, and the errors a formula can produce.
//!
//! Like Excel, dates are not a separate type — they are numbers shown through a
//! date number-format — so a `Value` is just empty, a number, text, a boolean,
//! or one of the seven canonical spreadsheet errors (plus circular reference).

use serde::{Deserialize, Serialize};
use std::fmt;

/// The canonical spreadsheet errors, rendered exactly as a user expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CellError {
    /// `#DIV/0!`
    Div0,
    /// `#REF!` — a reference to something that no longer exists
    Ref,
    /// `#VALUE!` — a value of the wrong type
    Value,
    /// `#NAME?` — an unknown function or name
    Name,
    /// `#N/A` — not available (e.g. a failed lookup)
    NA,
    /// `#NUM!` — an invalid number (e.g. SQRT of a negative)
    Num,
    /// `#NULL!` — an empty intersection of ranges
    Null,
    /// a circular reference (a cell that ultimately depends on itself)
    Circular,
}

impl fmt::Display for CellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            CellError::Div0 => "#DIV/0!",
            CellError::Ref | CellError::Circular => "#REF!",
            CellError::Value => "#VALUE!",
            CellError::Name => "#NAME?",
            CellError::NA => "#N/A",
            CellError::Num => "#NUM!",
            CellError::Null => "#NULL!",
        })
    }
}

/// A computed cell value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", content = "v")]
pub enum Value {
    Empty,
    Number(f64),
    Text(String),
    Bool(bool),
    Error(CellError),
}

impl Default for Value {
    fn default() -> Self {
        Value::Empty
    }
}

impl Value {
    pub fn is_error(&self) -> bool {
        matches!(self, Value::Error(_))
    }
    pub fn is_empty(&self) -> bool {
        matches!(self, Value::Empty)
    }

    /// Coerce to a number the way a formula context does: numbers pass through,
    /// booleans become 1/0, empty becomes 0, numeric text parses, everything
    /// else is `#VALUE!`, and an existing error propagates.
    pub fn as_number(&self) -> Result<f64, CellError> {
        match self {
            Value::Number(n) => Ok(*n),
            Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            Value::Empty => Ok(0.0),
            Value::Text(s) => s.trim().parse::<f64>().map_err(|_| CellError::Value),
            Value::Error(e) => Err(*e),
        }
    }

    /// The display-agnostic text of a value (no number formatting applied).
    pub fn as_text(&self) -> String {
        match self {
            Value::Empty => String::new(),
            Value::Number(n) => format_number(*n),
            Value::Text(s) => s.clone(),
            Value::Bool(b) => if *b { "TRUE".into() } else { "FALSE".into() },
            Value::Error(e) => e.to_string(),
        }
    }

    /// Truthiness for logical contexts (`IF`, `AND`, …).
    pub fn truthy(&self) -> Result<bool, CellError> {
        match self {
            Value::Bool(b) => Ok(*b),
            Value::Number(n) => Ok(*n != 0.0),
            Value::Empty => Ok(false),
            Value::Text(s) => match s.to_ascii_uppercase().as_str() {
                "TRUE" => Ok(true),
                "FALSE" => Ok(false),
                _ => Err(CellError::Value),
            },
            Value::Error(e) => Err(*e),
        }
    }

    /// Parse a raw literal a user typed into a cell (not a formula): a number,
    /// `TRUE`/`FALSE`, or text. Empty input is `Empty`.
    pub fn parse_literal(input: &str) -> Value {
        let t = input.trim();
        if t.is_empty() {
            return Value::Empty;
        }
        if let Ok(n) = t.parse::<f64>() {
            return Value::Number(n);
        }
        match t.to_ascii_uppercase().as_str() {
            "TRUE" => Value::Bool(true),
            "FALSE" => Value::Bool(false),
            _ => Value::Text(input.to_string()),
        }
    }
}

/// Render a float without a trailing `.0` for integers, the way a sheet does by
/// default before any number format is applied.
pub fn format_number(n: f64) -> String {
    if n.is_nan() {
        return CellError::Num.to_string();
    }
    if n.is_infinite() {
        return CellError::Div0.to_string();
    }
    if n == n.trunc() && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        let s = format!("{}", n);
        s
    }
}
