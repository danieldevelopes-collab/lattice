//! CSV export/import for the *active* sheet.
//!
//! Export walks the active sheet's used rectangle and writes each cell's
//! displayed text (`Value::as_text`), with empty strings for holes. Import is
//! the mirror image: every field becomes a literal cell input on a single
//! fresh sheet, so numbers and booleans are re-typed by the engine.

use csv::{ReaderBuilder, WriterBuilder};
use sheet_core::{CellRef, Workbook};

pub fn to_bytes(wb: &Workbook) -> Result<Vec<u8>, String> {
    let sheet = wb.active_sheet();
    let mut wtr = WriterBuilder::new().from_writer(Vec::new());

    if let Some(bounds) = sheet.used_bounds() {
        for row in 0..=bounds.row {
            let mut record: Vec<String> = Vec::with_capacity((bounds.col + 1) as usize);
            for col in 0..=bounds.col {
                let v = sheet.value(CellRef::new(col, row));
                record.push(v.as_text());
            }
            wtr.write_record(&record).map_err(|e| format!("csv write: {e}"))?;
        }
    }

    let inner = wtr.into_inner().map_err(|e| format!("csv flush: {e}"))?;
    Ok(inner)
}

pub fn from_bytes(data: &[u8]) -> Result<Workbook, String> {
    let mut wb = Workbook::new();
    let mut rdr = ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(data);

    {
        let sheet = wb.active_sheet_mut();
        for (row_idx, result) in rdr.records().enumerate() {
            let record = result.map_err(|e| format!("csv read: {e}"))?;
            for (col_idx, field) in record.iter().enumerate() {
                if field.is_empty() {
                    continue; // keep storage sparse — empties are no-ops
                }
                let at = CellRef::new(col_idx as u32, row_idx as u32);
                sheet.set(at, field.to_string());
            }
        }
    }

    Ok(wb)
}
