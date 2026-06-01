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

pub mod ast;
pub mod cellref;
pub mod eval;
pub mod format;
pub mod functions;
pub mod lexer;
pub mod model;
pub mod parser;
pub mod recalc;
pub mod style;
pub mod value;

pub use ast::Expr;
pub use cellref::{col_to_letters, letters_to_col, parse_a1, A1, CellRef, Range};
pub use eval::{eval, Context};
pub use format::{format_value, presets};
pub use model::{Cell, Sheet, Workbook, WorkbookContext};
pub use parser::parse_formula;
pub use recalc::recalculate;
pub use style::{HAlign, Style, StyleId, StyleTable, VAlign};
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

    // ---- M2: the formula engine --------------------------------------------

    use std::collections::HashMap;

    struct Grid(HashMap<CellRef, Value>);
    impl Grid {
        fn from(pairs: &[(&str, Value)]) -> Grid {
            let mut m = HashMap::new();
            for (a1, v) in pairs {
                m.insert(CellRef::parse(a1).unwrap(), v.clone());
            }
            Grid(m)
        }
    }
    impl crate::eval::Context for Grid {
        fn cell_value(&self, _sheet: Option<&str>, at: CellRef) -> Value {
            self.0.get(&at).cloned().unwrap_or(Value::Empty)
        }
    }
    fn empty() -> Grid {
        Grid(HashMap::new())
    }
    fn ev(f: &str, g: &Grid) -> Value {
        eval(&parse_formula(f).unwrap(), g)
    }
    fn num(f: &str, g: &Grid) -> f64 {
        match ev(f, g) {
            Value::Number(n) => n,
            other => panic!("expected a number from {f}, got {other:?}"),
        }
    }

    #[test]
    fn arithmetic_and_precedence() {
        let g = empty();
        assert_eq!(num("=1+2*3", &g), 7.0);
        assert_eq!(num("=(1+2)*3", &g), 9.0);
        assert_eq!(num("=2^3^2", &g), 64.0); // ^ is left-associative: (2^3)^2
        assert_eq!(num("=-2^2", &g), 4.0); // Excel: unary minus binds tighter than ^
        assert_eq!(num("=2^-2", &g), 0.25);
        assert_eq!(num("=10/4", &g), 2.5);
        assert_eq!(num("=10%", &g), 0.1);
        assert_eq!(num("=50%*4", &g), 2.0);
    }

    #[test]
    fn division_by_zero_propagates() {
        assert_eq!(ev("=1/0", &empty()), Value::Error(CellError::Div0));
        assert_eq!(ev("=5 + 1/0", &empty()), Value::Error(CellError::Div0));
    }

    #[test]
    fn concat_and_comparisons() {
        let g = empty();
        assert_eq!(ev("=\"a\"&\"b\"&1", &g), Value::Text("ab1".into()));
        assert_eq!(ev("=1<2", &g), Value::Bool(true));
        assert_eq!(ev("=2<=2", &g), Value::Bool(true));
        assert_eq!(ev("=3<>3", &g), Value::Bool(false));
        assert_eq!(ev("=\"x\"=\"X\"", &g), Value::Bool(true)); // case-insensitive text
    }

    #[test]
    fn references_resolve() {
        let g = Grid::from(&[("A1", Value::Number(10.0)), ("B1", Value::Number(5.0))]);
        assert_eq!(num("=A1+B1", &g), 15.0);
        assert_eq!(num("=A1*2", &g), 20.0);
        assert_eq!(num("=$A$1-B1", &g), 5.0);
    }

    #[test]
    fn aggregates_over_ranges() {
        let g = Grid::from(&[
            ("A1", Value::Number(1.0)),
            ("A2", Value::Number(2.0)),
            ("A3", Value::Number(3.0)),
        ]);
        assert_eq!(num("=SUM(A1:A3)", &g), 6.0);
        assert_eq!(num("=SUM(A1:A3, 10)", &g), 16.0);
        assert_eq!(num("=AVERAGE(A1:A3)", &g), 2.0);
        assert_eq!(num("=MAX(A1:A3)", &g), 3.0);
        assert_eq!(num("=MIN(A1:A3)", &g), 1.0);
        assert_eq!(num("=COUNT(A1:A3)", &g), 3.0);
        assert_eq!(num("=PRODUCT(A1:A3)", &g), 6.0);
    }

    #[test]
    fn text_in_ranges_ignored_by_sum() {
        let g = Grid::from(&[
            ("A1", Value::Number(1.0)),
            ("A2", Value::Text("x".into())),
            ("A3", Value::Number(2.0)),
        ]);
        assert_eq!(num("=SUM(A1:A3)", &g), 3.0);
        assert_eq!(num("=COUNT(A1:A3)", &g), 2.0);
        assert_eq!(num("=COUNTA(A1:A3)", &g), 3.0);
    }

    #[test]
    fn if_is_lazy() {
        let g = Grid::from(&[("A1", Value::Number(10.0))]);
        assert_eq!(ev("=IF(A1>5, \"big\", \"small\")", &g), Value::Text("big".into()));
        assert_eq!(num("=IF(FALSE, 1/0, 99)", &g), 99.0); // untaken 1/0 not evaluated
    }

    #[test]
    fn iferror_recovers() {
        assert_eq!(ev("=IFERROR(1/0, \"oops\")", &empty()), Value::Text("oops".into()));
        assert_eq!(num("=IFERROR(5, 0)", &empty()), 5.0);
    }

    #[test]
    fn math_functions() {
        let g = empty();
        assert_eq!(num("=ABS(-5)", &g), 5.0);
        assert_eq!(num("=SQRT(9)", &g), 3.0);
        assert_eq!(ev("=SQRT(-1)", &g), Value::Error(CellError::Num));
        assert_eq!(num("=MOD(7,3)", &g), 1.0);
        assert_eq!(num("=POWER(2,10)", &g), 1024.0);
        assert!((num("=ROUND(3.14159, 2)", &g) - 3.14).abs() < 1e-9);
        assert_eq!(num("=INT(3.9)", &g), 3.0);
    }

    #[test]
    fn text_functions() {
        let g = empty();
        assert_eq!(num("=LEN(\"hello\")", &g), 5.0);
        assert_eq!(ev("=UPPER(\"abc\")", &g), Value::Text("ABC".into()));
        assert_eq!(ev("=LOWER(\"ABC\")", &g), Value::Text("abc".into()));
        assert_eq!(ev("=CONCAT(\"a\", 1, \"b\")", &g), Value::Text("a1b".into()));
        assert_eq!(ev("=TRIM(\"  a   b \")", &g), Value::Text("a b".into()));
    }

    #[test]
    fn logical_functions() {
        let g = empty();
        assert_eq!(ev("=AND(1>0, 2>1)", &g), Value::Bool(true));
        assert_eq!(ev("=AND(1>0, 2<1)", &g), Value::Bool(false));
        assert_eq!(ev("=OR(FALSE, FALSE)", &g), Value::Bool(false));
        assert_eq!(ev("=NOT(1>0)", &g), Value::Bool(false));
    }

    #[test]
    fn unknown_names_and_error_propagation() {
        assert_eq!(ev("=FOO(1)", &empty()), Value::Error(CellError::Name));
        assert_eq!(ev("=undefined", &empty()), Value::Error(CellError::Name));
        assert_eq!(ev("=1 + SQRT(-1)", &empty()), Value::Error(CellError::Num));
    }

    #[test]
    fn nested_calls() {
        let g = Grid::from(&[
            ("A1", Value::Number(1.0)),
            ("A2", Value::Number(2.0)),
            ("A3", Value::Number(6.0)),
        ]);
        assert_eq!(num("=ROUND(AVERAGE(A1:A3), 0)", &g), 3.0);
        assert_eq!(num("=SUM(A1:A3) * 2 + MAX(A1:A3)", &g), 24.0);
    }

    #[test]
    fn workbook_evaluates_a_formula_cell() {
        let mut wb = Workbook::new();
        {
            let s = wb.active_sheet_mut();
            s.set_a1("A1", "10");
            s.set_a1("B1", "20");
            s.set_a1("C1", "=A1+B1");
        }
        let c1 = CellRef::parse("C1").unwrap();
        assert_eq!(wb.evaluate(0, c1), Value::Number(30.0));
        assert!(wb.active_sheet().get(c1).unwrap().ast.is_some());
    }
}
