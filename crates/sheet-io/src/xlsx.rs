//! XLSX export (via `rust_xlsxwriter`) and import (via `calamine`).
//!
//! On write, every non-empty cell is emitted as its computed value — a number,
//! string or boolean. Formula cells additionally carry their formula text (the
//! leading `=` stripped, as the writer wants) with the already-computed value
//! cached as the formula's result, so a reader that does not recalc still sees
//! the right number. On read, `calamine` hands back typed values which we map
//! straight onto literal cells.

use calamine::{Data, Reader, Xlsx};
use rust_xlsxwriter::{Formula, Workbook as XlsxWorkbook};
use sheet_core::{CellRef, Value, Workbook};
use std::io::Cursor;

/// XLSX columns are 16-bit; refuse anything wider rather than silently wrap.
const MAX_COL: u32 = u16::MAX as u32;

pub fn to_bytes(wb: &Workbook) -> Result<Vec<u8>, String> {
    let mut book = XlsxWorkbook::new();

    for sheet in &wb.sheets {
        let ws = book.add_worksheet();
        ws.set_name(&sheet.name).map_err(|e| format!("xlsx sheet name {:?}: {e}", sheet.name))?;

        for (at, cell) in sheet.iter() {
            if at.col > MAX_COL {
                return Err(format!(
                    "column {} exceeds the xlsx limit of {}",
                    at.col + 1,
                    MAX_COL + 1
                ));
            }
            let row = at.row;
            let col = at.col as u16;

            if cell.is_formula() {
                // Store formula text (without '=') plus the cached result so a
                // non-recalculating reader still sees a value.
                let body = cell.input.trim_start_matches('=').to_string();
                let result = cell.value.as_text();
                let formula = Formula::new(body).set_result(result);
                ws.write_formula(row, col, formula)
                    .map_err(|e| format!("xlsx write formula at {}: {e}", at.to_a1()))?;
                continue;
            }

            match &cell.value {
                Value::Number(n) => {
                    ws.write_number(row, col, *n)
                        .map_err(|e| format!("xlsx write number at {}: {e}", at.to_a1()))?;
                }
                Value::Bool(b) => {
                    ws.write_boolean(row, col, *b)
                        .map_err(|e| format!("xlsx write bool at {}: {e}", at.to_a1()))?;
                }
                Value::Text(s) => {
                    ws.write_string(row, col, s)
                        .map_err(|e| format!("xlsx write text at {}: {e}", at.to_a1()))?;
                }
                Value::Error(e) => {
                    // No native error-literal writer; record the display text.
                    ws.write_string(row, col, e.to_string())
                        .map_err(|e| format!("xlsx write error at {}: {e}", at.to_a1()))?;
                }
                Value::Empty => {
                    // A cell may exist with only an input and an Empty value
                    // (e.g. a formula that hasn't recalced). Preserve its text.
                    if !cell.input.is_empty() {
                        ws.write_string(row, col, &cell.input)
                            .map_err(|e| format!("xlsx write input at {}: {e}", at.to_a1()))?;
                    }
                }
            }
        }
    }

    book.save_to_buffer().map_err(|e| format!("xlsx save: {e}"))
}

pub fn from_bytes(data: &[u8]) -> Result<Workbook, String> {
    let cursor = Cursor::new(data.to_vec());
    let mut reader: Xlsx<_> = Xlsx::new(cursor).map_err(|e| format!("xlsx open: {e}"))?;

    let mut wb = Workbook::new();
    let sheet_names = reader.sheet_names().to_vec();
    if sheet_names.is_empty() {
        return Ok(wb);
    }

    for (i, name) in sheet_names.iter().enumerate() {
        let range = reader
            .worksheet_range(name)
            .map_err(|e| format!("xlsx read sheet {name:?}: {e}"))?;

        let idx = if i == 0 {
            wb.sheets[0].name = name.clone();
            0
        } else {
            wb.add_sheet(name.clone())
        };
        let sheet = wb.sheet_mut(idx).expect("just created");

        let Some((base_row, base_col)) = range.start() else {
            continue; // empty sheet
        };

        for (r, c, datum) in range.used_cells() {
            let at = CellRef::new(base_col + c as u32, base_row + r as u32);
            let input = data_to_input(datum);
            if !input.is_empty() {
                sheet.set(at, input);
            }
        }
    }

    Ok(wb)
}

/// Turn a calamine `Data` value into the raw input a cell should hold, so the
/// engine re-derives the same typed `Value`.
fn data_to_input(d: &Data) -> String {
    match d {
        Data::Empty => String::new(),
        Data::Int(i) => i.to_string(),
        Data::Float(f) => sheet_core::format_number(*f),
        Data::String(s) => s.clone(),
        Data::Bool(b) => {
            if *b {
                "TRUE".to_string()
            } else {
                "FALSE".to_string()
            }
        }
        Data::DateTime(dt) => dt.as_f64().to_string(),
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("{e:?}"),
    }
}
