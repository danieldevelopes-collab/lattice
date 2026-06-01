//! # sheet-core
//!
//! The spreadsheet engine behind **lattice**. It is pure Rust and entirely
//! independent of any UI, so every part of it — the model, the formula language
//! and the recalculation graph — can be tested on its own.
//!
//! Milestones: **M1 (this)** the cell/sheet/workbook model, A1 addressing, and
//! the value & error types · M2 the formula lexer / parser / evaluator · M3 the
//! dependency graph and incremental recalculation · M4+ the function library,
//! number formats and file interop.

pub mod cellref;
pub mod model;
pub mod value;

pub use cellref::{col_to_letters, letters_to_col, parse_a1, A1, CellRef, Range};
pub use model::{Cell, Sheet, Workbook};
pub use value::{format_number, CellError, Value};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_letters_round_trip() {
        for (col, letters) in [(0, "A"), (25, "Z"), (26, "AA"), (27, "AB"), (701, "ZZ"), (702, "AAA")] {
            assert_eq!(col_to_letters(col), letters, "col {col}");
            assert_eq!(letters_to_col(letters), Some(col), "letters {letters}");
        }
        assert_eq!(letters_to_col(""), None);
        assert_eq!(letters_to_col("A1"), None);
    }

    #[test]
    fn a1_parses_and_round_trips() {
        for (s, col, row) in [("A1", 0, 0), ("B3", 1, 2), ("Z10", 25, 9), ("AA1", 26, 0), ("ZZ100", 701, 99)] {
            let r = CellRef::parse(s).unwrap();
            assert_eq!((r.col, r.row), (col, row), "{s}");
            assert_eq!(r.to_a1(), s, "round trip {s}");
        }
    }

    #[test]
    fn absolute_markers_are_parsed() {
        let cases = [("A1", false, false), ("$A1", true, false), ("A$1", false, true), ("$A$1", true, true)];
        for (s, ac, ar) in cases {
            let p = parse_a1(s).unwrap();
            assert_eq!((p.abs_col, p.abs_row), (ac, ar), "{s}");
            assert_eq!(p.cell, CellRef::new(0, 0));
        }
    }

    #[test]
    fn a1_rejects_junk() {
        for bad in ["A", "1", "A1B", "A0", "", "AA", "$", "1A", "A 1"] {
            assert!(parse_a1(bad).is_none(), "should reject {bad:?}");
        }
    }

    #[test]
    fn ranges_normalise_and_iterate() {
        let r = Range::parse("C3:B2").unwrap(); // given reversed, must normalise
        assert_eq!(r.start, CellRef::new(1, 1));
        assert_eq!(r.end, CellRef::new(2, 2));
        assert_eq!(r.to_a1(), "B2:C3");
        assert_eq!((r.width(), r.height(), r.cell_count()), (2, 2, 4));
        let cells: Vec<_> = r.cells().collect();
        assert_eq!(
            cells,
            vec![CellRef::new(1, 1), CellRef::new(2, 1), CellRef::new(1, 2), CellRef::new(2, 2)]
        );
        assert!(r.contains(CellRef::new(2, 2)));
        assert!(!r.contains(CellRef::new(3, 2)));
    }

    #[test]
    fn cellref_offset() {
        let a = CellRef::new(2, 3);
        assert_eq!(a.offset(1, -1), Some(CellRef::new(3, 2)));
        assert_eq!(CellRef::new(0, 0).offset(-1, 0), None);
    }

    #[test]
    fn literals_parse() {
        assert_eq!(Value::parse_literal("42"), Value::Number(42.0));
        assert_eq!(Value::parse_literal("3.14"), Value::Number(3.14));
        assert_eq!(Value::parse_literal("-5"), Value::Number(-5.0));
        assert_eq!(Value::parse_literal("TRUE"), Value::Bool(true));
        assert_eq!(Value::parse_literal("false"), Value::Bool(false));
        assert_eq!(Value::parse_literal("hello"), Value::Text("hello".into()));
        assert_eq!(Value::parse_literal(""), Value::Empty);
    }

    #[test]
    fn number_coercions() {
        assert_eq!(Value::Bool(true).as_number(), Ok(1.0));
        assert_eq!(Value::Empty.as_number(), Ok(0.0));
        assert_eq!(Value::Text("5".into()).as_number(), Ok(5.0));
        assert_eq!(Value::Text("x".into()).as_number(), Err(CellError::Value));
        assert_eq!(Value::Error(CellError::Div0).as_number(), Err(CellError::Div0));
    }

    #[test]
    fn error_display_matches_excel() {
        assert_eq!(CellError::Div0.to_string(), "#DIV/0!");
        assert_eq!(CellError::Value.to_string(), "#VALUE!");
        assert_eq!(CellError::Name.to_string(), "#NAME?");
        assert_eq!(CellError::NA.to_string(), "#N/A");
        assert_eq!(CellError::Num.to_string(), "#NUM!");
        assert_eq!(CellError::Null.to_string(), "#NULL!");
        assert_eq!(CellError::Circular.to_string(), "#REF!");
    }

    #[test]
    fn number_formatting_is_clean() {
        assert_eq!(format_number(42.0), "42");
        assert_eq!(format_number(-7.0), "-7");
        assert_eq!(format_number(3.5), "3.5");
    }

    #[test]
    fn sheet_set_get_and_sparsity() {
        let mut s = Sheet::new("Sheet1");
        s.set_a1("A1", "42");
        s.set_a1("A2", "hello");
        s.set_a1("A3", "=A1+1");
        assert_eq!(s.value(CellRef::parse("A1").unwrap()), Value::Number(42.0));
        assert_eq!(s.value(CellRef::parse("A2").unwrap()), Value::Text("hello".into()));
        // a formula is stored as input with an Empty value until M2/M3
        let a3 = s.get(CellRef::parse("A3").unwrap()).unwrap();
        assert!(a3.is_formula());
        assert_eq!(a3.value, Value::Empty);
        // untouched cell
        assert_eq!(s.value(CellRef::parse("Z99").unwrap()), Value::Empty);
        assert_eq!(s.len(), 3);
        assert_eq!(s.used_bounds(), Some(CellRef::new(0, 2)));
        // clearing keeps storage sparse
        s.set_a1("A2", "");
        assert_eq!(s.len(), 2);
        assert!(s.get(CellRef::parse("A2").unwrap()).is_none());
    }

    #[test]
    fn workbook_basics() {
        let mut wb = Workbook::new();
        assert_eq!(wb.sheets.len(), 1);
        assert_eq!(wb.active_sheet().name, "Sheet1");
        let i = wb.add_sheet("Budget");
        assert_eq!(i, 1);
        assert_eq!(wb.index_of("Budget"), Some(1));
        wb.sheet_mut(1).unwrap().set_a1("A1", "10");
        assert_eq!(wb.sheet(1).unwrap().value(CellRef::new(0, 0)), Value::Number(10.0));
    }
}
