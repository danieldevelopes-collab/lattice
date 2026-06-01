//! A1-style cell addressing — the coordinate system every spreadsheet speaks.
//!
//! Internally a cell is a zero-based `(col, row)`; on screen and in formulas it
//! is `A1` (column letters + a one-based row). This module is the single place
//! that knows how to translate between the two, how to parse the `$` absolute
//! markers that matter when you copy a formula, and how to handle ranges like
//! `A1:B10`.

use serde::{Deserialize, Serialize};

/// A zero-based cell coordinate. `A1` is `{ col: 0, row: 0 }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CellRef {
    pub col: u32,
    pub row: u32,
}

impl CellRef {
    pub fn new(col: u32, row: u32) -> Self {
        CellRef { col, row }
    }
    /// The `A1` spelling, e.g. `{0,0}` -> `"A1"`, `{26,0}` -> `"AA1"`.
    pub fn to_a1(self) -> String {
        format!("{}{}", col_to_letters(self.col), self.row + 1)
    }
    /// Parse an `A1` reference (the `$` markers are accepted and ignored here;
    /// use [`parse_a1`] when you need the absolute flags).
    pub fn parse(s: &str) -> Option<CellRef> {
        parse_a1(s).map(|p| p.cell)
    }
    /// Shift by a signed delta, returning `None` on underflow/overflow.
    pub fn offset(self, dcol: i64, drow: i64) -> Option<CellRef> {
        let col = (self.col as i64 + dcol).try_into().ok()?;
        let row = (self.row as i64 + drow).try_into().ok()?;
        Some(CellRef { col, row })
    }
}

/// An `A1` reference plus whether its column/row were written absolute (`$`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct A1 {
    pub cell: CellRef,
    pub abs_col: bool,
    pub abs_row: bool,
}

/// `0 -> "A"`, `25 -> "Z"`, `26 -> "AA"`, `701 -> "ZZ"` … (bijective base-26).
pub fn col_to_letters(mut col: u32) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (col % 26) as u8);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap()
}

/// Inverse of [`col_to_letters`]: `"A" -> 0`, `"AA" -> 26`. `None` if not letters.
pub fn letters_to_col(s: &str) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    let mut col: u32 = 0;
    for c in s.chars() {
        if !c.is_ascii_alphabetic() {
            return None;
        }
        let d = (c.to_ascii_uppercase() as u32) - ('A' as u32) + 1;
        col = col.checked_mul(26)?.checked_add(d)?;
    }
    Some(col - 1)
}

/// Parse `A1`, `$A$1`, `B$3`, `AA12`, … into a cell plus its `$` flags.
pub fn parse_a1(s: &str) -> Option<A1> {
    let s = s.trim();
    let b = s.as_bytes();
    let mut i = 0;

    let abs_col = b.get(i) == Some(&b'$');
    if abs_col {
        i += 1;
    }
    let lstart = i;
    while i < b.len() && b[i].is_ascii_alphabetic() {
        i += 1;
    }
    if i == lstart {
        return None;
    }
    let letters = &s[lstart..i];

    let abs_row = b.get(i) == Some(&b'$');
    if abs_row {
        i += 1;
    }
    let dstart = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == dstart || i != b.len() {
        return None; // trailing junk or no digits
    }
    let row1: u32 = s[dstart..i].parse().ok()?;
    if row1 == 0 {
        return None;
    }
    Some(A1 {
        cell: CellRef::new(letters_to_col(letters)?, row1 - 1),
        abs_col,
        abs_row,
    })
}

/// A rectangular block of cells, e.g. `A1:B10`. Always stored normalised so
/// `start` is the top-left and `end` the bottom-right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub start: CellRef,
    pub end: CellRef,
}

impl Range {
    pub fn new(a: CellRef, b: CellRef) -> Self {
        Range {
            start: CellRef::new(a.col.min(b.col), a.row.min(b.row)),
            end: CellRef::new(a.col.max(b.col), a.row.max(b.row)),
        }
    }
    pub fn parse(s: &str) -> Option<Range> {
        let (a, b) = s.split_once(':')?;
        Some(Range::new(parse_a1(a)?.cell, parse_a1(b)?.cell))
    }
    pub fn to_a1(self) -> String {
        format!("{}:{}", self.start.to_a1(), self.end.to_a1())
    }
    pub fn contains(self, c: CellRef) -> bool {
        c.col >= self.start.col && c.col <= self.end.col && c.row >= self.start.row && c.row <= self.end.row
    }
    pub fn width(self) -> u32 {
        self.end.col - self.start.col + 1
    }
    pub fn height(self) -> u32 {
        self.end.row - self.start.row + 1
    }
    pub fn cell_count(self) -> u64 {
        self.width() as u64 * self.height() as u64
    }
    /// Iterate the cells row-major (the order `SUM` and friends expect).
    pub fn cells(self) -> impl Iterator<Item = CellRef> {
        let (c0, c1, r0, r1) = (self.start.col, self.end.col, self.start.row, self.end.row);
        (r0..=r1).flat_map(move |r| (c0..=c1).map(move |c| CellRef::new(c, r)))
    }
}
