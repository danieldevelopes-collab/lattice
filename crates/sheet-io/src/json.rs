//! Native JSON persistence — the only fully lossless format.
//!
//! The engine's sheets are sparse `HashMap`s that don't implement `serde`
//! directly, so we project each sheet to a list of `{a1, input}` pairs (the raw
//! text the user typed) and rebuild the workbook by replaying those inputs
//! through `set_a1`. That means formulas and literals come back byte-for-byte,
//! and every value is recomputed by the engine on load.

use serde::{Deserialize, Serialize};
use sheet_core::Workbook;

#[derive(Serialize, Deserialize)]
struct CellDto {
    a1: String,
    input: String,
}

#[derive(Serialize, Deserialize)]
struct SheetDto {
    name: String,
    cells: Vec<CellDto>,
}

#[derive(Serialize, Deserialize)]
struct WorkbookDto {
    /// Bumped if the on-disk shape ever changes; lets a future loader adapt.
    version: u32,
    active: usize,
    sheets: Vec<SheetDto>,
}

const VERSION: u32 = 1;

fn to_dto(wb: &Workbook) -> WorkbookDto {
    let sheets = wb
        .sheets
        .iter()
        .map(|s| {
            // Sort by (row, col) so the serialised form is stable across runs
            // even though the underlying storage is an unordered map.
            let mut cells: Vec<CellDto> = s
                .iter()
                .map(|(at, cell)| CellDto { a1: at.to_a1(), input: cell.input.clone() })
                .collect();
            cells.sort_by_key(|c| {
                sheet_core::CellRef::parse(&c.a1).map(|r| (r.row, r.col)).unwrap_or((u32::MAX, u32::MAX))
            });
            SheetDto { name: s.name.clone(), cells }
        })
        .collect();
    WorkbookDto { version: VERSION, active: wb.active, sheets }
}

fn from_dto(dto: WorkbookDto) -> Workbook {
    let mut wb = Workbook::new();
    // `Workbook::new` already has one sheet ("Sheet1"); reuse it for the first
    // serialised sheet and add the rest.
    for (i, sd) in dto.sheets.iter().enumerate() {
        let idx = if i == 0 {
            wb.sheets[0].name = sd.name.clone();
            0
        } else {
            wb.add_sheet(sd.name.clone())
        };
        let sheet = wb.sheet_mut(idx).expect("just created");
        for c in &sd.cells {
            sheet.set_a1(&c.a1, c.input.clone());
        }
    }
    if dto.sheets.is_empty() {
        // Degenerate input: keep the default lone sheet.
    }
    wb.active = dto.active.min(wb.sheets.len().saturating_sub(1));
    wb
}

pub fn to_bytes(wb: &Workbook) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(&to_dto(wb)).map_err(|e| format!("json encode: {e}"))
}

pub fn from_bytes(data: &[u8]) -> Result<Workbook, String> {
    let dto: WorkbookDto = serde_json::from_slice(data).map_err(|e| format!("json decode: {e}"))?;
    Ok(from_dto(dto))
}
