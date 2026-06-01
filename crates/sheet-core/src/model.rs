//! The document model: cells, sheets, a workbook.
//!
//! Storage is **sparse** — only cells a user has touched exist, so an
//! "infinite" grid costs nothing until it's filled. In M1 a cell stores its raw
//! input and, for plain literals, the parsed value; formulas (`=…`) are kept as
//! input with an `Empty` value until the formula engine (M2) and recalculation
//! (M3) bring them to life.

use crate::ast::Expr;
use crate::cellref::CellRef;
use crate::value::{CellError, Value};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One cell: what was typed, and what it currently evaluates to.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cell {
    /// Exactly what the user typed: `"42"`, `"hello"`, `"=A1+B2"`.
    pub input: String,
    /// The computed value shown in the grid.
    pub value: Value,
    /// The parsed formula, when `input` begins with `=`. Re-parsed from `input`
    /// on load, so it is not serialized; `None` for plain literals.
    #[serde(skip)]
    pub ast: Option<Expr>,
}

impl Cell {
    pub fn is_formula(&self) -> bool {
        self.input.starts_with('=') && self.input.len() > 1
    }
}

/// One sheet: a named, sparse grid of cells.
#[derive(Debug, Clone)]
pub struct Sheet {
    pub name: String,
    cells: HashMap<CellRef, Cell>,
}

impl Sheet {
    pub fn new(name: impl Into<String>) -> Self {
        Sheet { name: name.into(), cells: HashMap::new() }
    }

    /// Set a cell from raw input. Empty input clears the cell (keeping storage
    /// sparse). Literals are parsed immediately; formulas keep `Empty` for now.
    pub fn set(&mut self, at: CellRef, input: impl Into<String>) {
        let input = input.into();
        if input.is_empty() {
            self.cells.remove(&at);
            return;
        }
        let (value, ast) = if input.starts_with('=') && input.len() > 1 {
            match crate::parser::parse_formula(&input) {
                // a freshly parsed formula has no value until recalc (M3) runs
                Ok(ast) => (Value::Empty, Some(ast)),
                Err(_) => (Value::Error(CellError::Value), None),
            }
        } else {
            (Value::parse_literal(&input), None)
        };
        self.cells.insert(at, Cell { input, value, ast });
    }

    pub fn set_a1(&mut self, a1: &str, input: impl Into<String>) -> bool {
        match CellRef::parse(a1) {
            Some(r) => {
                self.set(r, input);
                true
            }
            None => false,
        }
    }

    pub fn get(&self, at: CellRef) -> Option<&Cell> {
        self.cells.get(&at)
    }

    /// The value at a coordinate; `Empty` for never-touched cells.
    pub fn value(&self, at: CellRef) -> Value {
        self.cells.get(&at).map(|c| c.value.clone()).unwrap_or(Value::Empty)
    }

    pub fn clear(&mut self, at: CellRef) {
        self.cells.remove(&at);
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&CellRef, &Cell)> {
        self.cells.iter()
    }

    /// The bottom-right corner of the used area (`None` if the sheet is empty).
    pub fn used_bounds(&self) -> Option<CellRef> {
        let mut max: Option<CellRef> = None;
        for r in self.cells.keys() {
            max = Some(match max {
                Some(m) => CellRef::new(m.col.max(r.col), m.row.max(r.row)),
                None => *r,
            });
        }
        max
    }
}

/// A workbook: an ordered set of sheets and which one is active.
#[derive(Debug, Clone)]
pub struct Workbook {
    pub sheets: Vec<Sheet>,
    pub active: usize,
}

impl Default for Workbook {
    fn default() -> Self {
        Workbook { sheets: vec![Sheet::new("Sheet1")], active: 0 }
    }
}

impl Workbook {
    pub fn new() -> Self {
        Workbook::default()
    }

    pub fn add_sheet(&mut self, name: impl Into<String>) -> usize {
        self.sheets.push(Sheet::new(name));
        self.sheets.len() - 1
    }

    pub fn sheet(&self, idx: usize) -> Option<&Sheet> {
        self.sheets.get(idx)
    }
    pub fn sheet_mut(&mut self, idx: usize) -> Option<&mut Sheet> {
        self.sheets.get_mut(idx)
    }

    pub fn sheet_by_name(&self, name: &str) -> Option<&Sheet> {
        self.sheets.iter().find(|s| s.name == name)
    }
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.sheets.iter().position(|s| s.name == name)
    }

    pub fn active_sheet(&self) -> &Sheet {
        &self.sheets[self.active]
    }
    pub fn active_sheet_mut(&mut self) -> &mut Sheet {
        &mut self.sheets[self.active]
    }

    /// Evaluate a single cell's formula against the *current* cell values.
    /// This does not order recalculation (that is M3) — it simply computes one
    /// cell. Literals return their stored value.
    pub fn evaluate(&self, sheet_idx: usize, at: CellRef) -> Value {
        let cell = match self.sheet(sheet_idx).and_then(|s| s.get(at)) {
            Some(c) => c,
            None => return Value::Empty,
        };
        match &cell.ast {
            Some(ast) => crate::eval::eval(ast, &WorkbookContext { wb: self, sheet: sheet_idx }),
            None => cell.value.clone(),
        }
    }
}

/// A read-only evaluation context over a workbook: unqualified references
/// resolve against `sheet`, qualified ones (`Sheet2!A1`) against that sheet.
pub struct WorkbookContext<'a> {
    pub wb: &'a Workbook,
    pub sheet: usize,
}

impl crate::eval::Context for WorkbookContext<'_> {
    fn cell_value(&self, sheet: Option<&str>, at: CellRef) -> Value {
        let idx = match sheet {
            None => self.sheet,
            Some(name) => match self.wb.index_of(name) {
                Some(i) => i,
                None => return Value::Error(CellError::Ref),
            },
        };
        self.wb.sheet(idx).map(|s| s.value(at)).unwrap_or(Value::Empty)
    }
}
