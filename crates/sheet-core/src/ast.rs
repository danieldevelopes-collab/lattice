//! The formula syntax tree.
//!
//! A parsed formula is a tree of `Expr`. References keep their `$` absolute
//! flags and optional sheet qualifier so that, later, copying a formula can
//! adjust the *relative* parts while leaving the absolute ones fixed.

use crate::cellref::{CellRef, Range};
use crate::value::CellError;
use serde::{Deserialize, Serialize};

/// A single reference component: a coordinate plus whether each axis was
/// written absolute (`$`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefPart {
    pub cell: CellRef,
    pub abs_col: bool,
    pub abs_row: bool,
}

/// A cell reference, possibly qualified by a sheet (`Sheet2!A1`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reference {
    pub sheet: Option<String>,
    pub part: RefPart,
}

/// A range reference (`A1:B10`, `Sheet2!A1:B10`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RangeRef {
    pub sheet: Option<String>,
    pub a: RefPart,
    pub b: RefPart,
}

impl RangeRef {
    /// The normalised rectangle this range covers.
    pub fn range(&self) -> Range {
        Range::new(self.a.cell, self.b.cell)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Concat,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnaryOp {
    Neg,
    Pos,
    Percent,
}

/// A node in a parsed formula.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Expr {
    Number(f64),
    Text(String),
    Bool(bool),
    Error(CellError),
    Ref(Reference),
    Range(RangeRef),
    /// A bare identifier that isn't a reference — a named range, resolved later;
    /// until then it evaluates to `#NAME?`.
    Name(String),
    Unary(UnaryOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Func(String, Vec<Expr>),
}
