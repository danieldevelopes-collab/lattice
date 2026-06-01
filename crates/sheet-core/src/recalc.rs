//! Incremental recalculation — dependency graph + topological recompute.
//!
//! A sheet's formula cells form a directed graph: an edge `p -> c` means cell
//! `c` reads cell `p`, so `p` must be computed first. We snapshot the formulas,
//! extract each one's *same-sheet* precedents, order the cells with a
//! topological sort (Kahn's algorithm), and evaluate them in that order so each
//! dependent always reads freshly computed inputs. Cells caught in a cycle
//! cannot be ordered and are flagged `#REF!` (a circular reference).
//!
//! Sheet-qualified references (`Sheet2!A1`) are treated as external inputs: they
//! are read during evaluation but never create an ordering edge here, since this
//! pass only sequences the cells that live on `sheet`.

use crate::ast::Expr;
use crate::cellref::CellRef;
use crate::model::{Workbook, WorkbookContext};
use crate::value::{CellError, Value};
use std::collections::{HashMap, HashSet, VecDeque};

/// Recompute every formula cell on `sheet`, in dependency order.
///
/// Each formula is evaluated against the current cell values and its result is
/// written back before its dependents are visited. Any formula cells that form
/// a cycle are set to `Value::Error(CellError::Circular)`.
pub fn recalculate(wb: &mut Workbook, sheet: usize) {
    // Nothing to do if the index is out of range.
    if wb.sheet(sheet).is_none() {
        return;
    }

    // 1. Snapshot the formula cells, cloning each `Expr` so we hold no borrow on
    //    `wb` while we later mutate it. `formula_cells` order is unspecified
    //    (it iterates a HashMap); the topological sort imposes the real order.
    let formulas: Vec<(CellRef, Expr)> = wb
        .sheet(sheet)
        .expect("checked above")
        .formula_cells()
        .map(|(at, ast)| (at, ast.clone()))
        .collect();

    // The set of cells that actually hold formulas — only these can be ordering
    // dependencies; a precedent that is a plain literal is just an input value.
    let formula_set: HashSet<CellRef> = formulas.iter().map(|(at, _)| *at).collect();

    // 2 + 3. Build the dependency graph among the formula cells.
    //
    //   successors[p] = formula cells that read p (edges p -> c)
    //   indegree[c]   = number of formula precedents c still waits on
    //
    // Every formula starts in `indegree` (with 0) so isolated cells — those with
    // no formula precedents — are picked up immediately by Kahn's algorithm.
    let mut successors: HashMap<CellRef, Vec<CellRef>> = HashMap::new();
    let mut indegree: HashMap<CellRef, usize> = formulas.iter().map(|(at, _)| (*at, 0)).collect();

    for (cell, expr) in &formulas {
        // Collect the cell's same-sheet precedents, de-duplicated so a formula
        // like `=A1+A1` (or overlapping ranges) counts A1 as a single edge.
        let mut precedents: HashSet<CellRef> = HashSet::new();
        collect_precedents(expr, &mut precedents);

        for p in precedents {
            // Only precedents that are themselves formula cells create ordering
            // edges; self-references (a 1-cell cycle) are kept so the cycle is
            // detected and the cell is flagged, rather than silently computed.
            if formula_set.contains(&p) {
                successors.entry(p).or_default().push(*cell);
                *indegree.entry(*cell).or_insert(0) += 1;
            }
        }
    }

    // 4. Topological sort with Kahn's algorithm: repeatedly take a cell with no
    //    remaining unresolved precedents, then relax its successors' indegrees.
    let mut queue: VecDeque<CellRef> =
        indegree.iter().filter(|(_, &d)| d == 0).map(|(c, _)| *c).collect();
    let mut order: Vec<CellRef> = Vec::with_capacity(formulas.len());

    while let Some(cell) = queue.pop_front() {
        order.push(cell);
        if let Some(deps) = successors.get(&cell) {
            for &next in deps {
                if let Some(d) = indegree.get_mut(&next) {
                    *d -= 1;
                    if *d == 0 {
                        queue.push_back(next);
                    }
                }
            }
        }
    }

    // Anything that never reached indegree 0 is part of (or downstream of) a
    // cycle. Mark those cells circular; they are excluded from evaluation.
    let ordered: HashSet<CellRef> = order.iter().copied().collect();
    let mut cyclic: HashSet<CellRef> = HashSet::new();
    for (cell, _) in &formulas {
        if !ordered.contains(cell) {
            wb.sheet_mut(sheet)
                .expect("checked above")
                .set_computed(*cell, Value::Error(CellError::Circular));
            cyclic.insert(*cell);
        }
    }

    // 5. Evaluate the acyclic cells in topological order, writing each result
    //    back before its dependents run so they read fresh values. A map from
    //    cell -> expr lets us look up each cell's formula by its sorted position.
    let by_cell: HashMap<CellRef, &Expr> = formulas.iter().map(|(at, e)| (*at, e)).collect();

    for cell in &order {
        // Should always be present, but skip defensively rather than panic.
        let expr = match by_cell.get(cell) {
            Some(e) => *e,
            None => continue,
        };
        // The immutable borrow (the eval context) is confined to this block, so
        // it has ended by the time we take the mutable borrow to write the value.
        let v = {
            let ctx = WorkbookContext { wb, sheet };
            crate::eval::eval(expr, &ctx)
        };
        wb.sheet_mut(sheet).expect("checked above").set_computed(*cell, v);
    }

    // `cyclic` is retained for clarity/debugging symmetry with `ordered`; it has
    // no further use once the circular cells are written above.
    let _ = cyclic;
}

/// Walk an expression and collect every *same-sheet* cell it reads into `out`.
///
/// Same-sheet means the reference carries no sheet qualifier (`sheet.is_none()`):
/// a bare `A1` or `A1:B3`. Sheet-qualified references (`Sheet2!A1`) are external
/// inputs and are deliberately ignored, since this graph only orders the cells
/// that live on the sheet being recalculated. Compound expressions recurse.
fn collect_precedents(expr: &Expr, out: &mut HashSet<CellRef>) {
    match expr {
        Expr::Ref(reference) => {
            if reference.sheet.is_none() {
                out.insert(reference.part.cell);
            }
        }
        Expr::Range(range_ref) => {
            if range_ref.sheet.is_none() {
                for cell in range_ref.range().cells() {
                    out.insert(cell);
                }
            }
        }
        Expr::Unary(_, inner) => collect_precedents(inner, out),
        Expr::Binary(_, a, b) => {
            collect_precedents(a, out);
            collect_precedents(b, out);
        }
        Expr::Func(_, args) => {
            for arg in args {
                collect_precedents(arg, out);
            }
        }
        // Leaves with no cell references: literals and unresolved names.
        Expr::Number(_)
        | Expr::Text(_)
        | Expr::Bool(_)
        | Expr::Error(_)
        | Expr::Name(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Workbook;

    /// Read the computed value at an A1 address on sheet 0.
    fn val(wb: &Workbook, a1: &str) -> Value {
        wb.sheet(0).unwrap().value(CellRef::parse(a1).unwrap())
    }

    #[test]
    fn chain_computes_in_dependency_order() {
        let mut wb = Workbook::new();
        {
            let s = wb.active_sheet_mut();
            s.set_a1("A1", "1");
            s.set_a1("A2", "=A1+1");
            s.set_a1("A3", "=A2*2");
        }
        recalculate(&mut wb, 0);
        assert_eq!(val(&wb, "A2"), Value::Number(2.0));
        assert_eq!(val(&wb, "A3"), Value::Number(4.0));
    }

    #[test]
    fn re_edit_propagates_through_chain() {
        let mut wb = Workbook::new();
        {
            let s = wb.active_sheet_mut();
            s.set_a1("A1", "1");
            s.set_a1("A2", "=A1+1");
            s.set_a1("A3", "=A2*2");
        }
        recalculate(&mut wb, 0);

        // Edit the root input and recompute: the change must flow downstream.
        wb.active_sheet_mut().set_a1("A1", "10");
        recalculate(&mut wb, 0);
        assert_eq!(val(&wb, "A2"), Value::Number(11.0));
        assert_eq!(val(&wb, "A3"), Value::Number(22.0));
    }

    #[test]
    fn cycle_marks_both_cells_circular() {
        let mut wb = Workbook::new();
        {
            let s = wb.active_sheet_mut();
            s.set_a1("A1", "=A2");
            s.set_a1("A2", "=A1");
        }
        recalculate(&mut wb, 0);
        assert_eq!(val(&wb, "A1"), Value::Error(CellError::Circular));
        assert_eq!(val(&wb, "A2"), Value::Error(CellError::Circular));
    }

    #[test]
    fn self_reference_is_circular() {
        let mut wb = Workbook::new();
        wb.active_sheet_mut().set_a1("A1", "=A1+1");
        recalculate(&mut wb, 0);
        assert_eq!(val(&wb, "A1"), Value::Error(CellError::Circular));
    }

    #[test]
    fn acyclic_cells_survive_when_others_cycle() {
        // An independent chain must still compute even though A1/A2 form a cycle.
        let mut wb = Workbook::new();
        {
            let s = wb.active_sheet_mut();
            s.set_a1("A1", "=A2");
            s.set_a1("A2", "=A1");
            s.set_a1("B1", "5");
            s.set_a1("B2", "=B1*3");
        }
        recalculate(&mut wb, 0);
        assert_eq!(val(&wb, "A1"), Value::Error(CellError::Circular));
        assert_eq!(val(&wb, "A2"), Value::Error(CellError::Circular));
        assert_eq!(val(&wb, "B2"), Value::Number(15.0));
    }

    #[test]
    fn range_dependency_orders_correctly() {
        // A4 sums A1:A3 where A3 is itself a formula; ordering must put A3 first.
        let mut wb = Workbook::new();
        {
            let s = wb.active_sheet_mut();
            s.set_a1("A1", "1");
            s.set_a1("A2", "2");
            s.set_a1("A3", "=A1+A2");
            s.set_a1("A4", "=SUM(A1:A3)");
        }
        recalculate(&mut wb, 0);
        assert_eq!(val(&wb, "A3"), Value::Number(3.0));
        assert_eq!(val(&wb, "A4"), Value::Number(6.0));
    }
}
