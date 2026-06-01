// lattice — the Tauri shell.
//
// This binary is deliberately thin. It opens a window, serves the web UI, and
// owns the one authoritative `Workbook`. Four commands hand work straight to the
// Rust engine: `new_workbook` resets it, `set_cell` applies an edit and
// recalculates, `open_workbook` / `save_workbook` bridge to the file formats.
// All the real logic — the formula language, the dependency-ordered
// recalculation and every importer/exporter — lives in well-tested crates
// (`sheet-core`, `sheet-io`), so this file is only the bridge between them and
// the canvas grid in the browser.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;

use serde::Serialize;
use sheet_core::{CellRef, Sheet, Workbook};
use sheet_io::Format;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

/// The backend's single source of truth: the open workbook, behind a mutex so
/// the synchronous Tauri commands can borrow it one at a time.
struct AppState {
    wb: Mutex<Workbook>,
}

// ---------------------------------------------------------------------------
// Snapshot DTOs — the exact shapes the web UI (src/app.js) consumes.
// ---------------------------------------------------------------------------

/// One cell on the wire: its address, the raw input the user typed, and the
/// computed text to paint.
#[derive(Serialize)]
struct CellSnap {
    a1: String,
    input: String,
    display: String,
}

/// A single sheet projected to its live (non-empty) cells.
#[derive(Serialize)]
struct SheetSnap {
    name: String,
    cells: Vec<CellSnap>,
}

/// A whole workbook: every sheet plus which one is active.
#[derive(Serialize)]
struct WorkbookSnap {
    active: usize,
    sheets: Vec<SheetSnap>,
}

/// Project one sheet to `{name, cells:[{a1, input, display}]}`, sorted by
/// (row, col) so the wire form — and any screenshot taken from it — is stable
/// even though the underlying storage is an unordered map.
fn snapshot_sheet(sheet: &Sheet) -> SheetSnap {
    let mut cells: Vec<CellSnap> = sheet
        .iter()
        .map(|(at, cell)| CellSnap {
            a1: at.to_a1(),
            input: cell.input.clone(),
            display: cell.value.as_text(),
        })
        .collect();
    cells.sort_by_key(|c| {
        CellRef::parse(&c.a1)
            .map(|r| (r.row, r.col))
            .unwrap_or((u32::MAX, u32::MAX))
    });
    SheetSnap { name: sheet.name.clone(), cells }
}

fn snapshot_workbook(wb: &Workbook) -> WorkbookSnap {
    WorkbookSnap {
        active: wb.active,
        sheets: wb.sheets.iter().map(snapshot_sheet).collect(),
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Reset to a fresh, empty workbook and return its full snapshot. Called when
/// the UI boots and when the user picks "New".
#[tauri::command]
fn new_workbook(state: State<AppState>) -> WorkbookSnap {
    let mut wb = state.wb.lock().unwrap();
    *wb = Workbook::new();
    snapshot_workbook(&wb)
}

/// The result of one edit: the cells that may have changed.
#[derive(Serialize)]
struct SetResult {
    cells: Vec<CellSnap>,
}

/// Apply a single edit, recalculate the sheet in dependency order, and return
/// the sheet's live cells (so every formula that the edit touched repaints with
/// a fresh value). If the edit cleared the cell, an explicit empty entry is
/// appended so the UI drops it from its local model.
#[tauri::command]
fn set_cell(
    state: State<AppState>,
    sheet: usize,
    a1: String,
    input: String,
) -> Result<SetResult, String> {
    let at = CellRef::parse(&a1).ok_or_else(|| format!("bad cell reference: {a1}"))?;

    let mut wb = state.wb.lock().unwrap();
    {
        let s = wb
            .sheet_mut(sheet)
            .ok_or_else(|| format!("no sheet at index {sheet}"))?;
        s.set(at, input);
    }
    wb.recalculate(sheet);

    let s = wb
        .sheet(sheet)
        .ok_or_else(|| format!("no sheet at index {sheet}"))?;
    let mut snap = snapshot_sheet(s);
    if s.get(at).is_none() {
        // The cell was cleared: send a blank so the front-end deletes it.
        snap.cells.push(CellSnap {
            a1: at.to_a1(),
            input: String::new(),
            display: String::new(),
        });
    }
    Ok(SetResult { cells: snap.cells })
}

/// What `open_workbook` hands back: the file it read and the active sheet.
#[derive(Serialize)]
struct OpenResult {
    path: String,
    snapshot: SheetSnap,
}

/// Show a native open dialog, load the chosen file (xlsx / ods / csv / json),
/// recalculate every sheet, install it as the live workbook, and return the
/// active sheet's snapshot. `Ok(None)` when the user cancels.
#[tauri::command]
fn open_workbook(app: AppHandle, state: State<AppState>) -> Result<Option<OpenResult>, String> {
    let picked = app
        .dialog()
        .file()
        .add_filter("Spreadsheets", &["xlsx", "ods", "csv", "json"])
        .add_filter("Excel workbook", &["xlsx"])
        .add_filter("OpenDocument spreadsheet", &["ods"])
        .add_filter("CSV", &["csv"])
        .add_filter("Lattice workbook", &["json"])
        .blocking_pick_file();

    let Some(file) = picked else {
        return Ok(None);
    };
    let path = file.into_path().map_err(|e| e.to_string())?;
    let path_str = path.to_string_lossy().into_owned();

    let mut loaded = sheet_io::load(&path_str)?;
    loaded.recalculate_all();
    let active = loaded.active.min(loaded.sheets.len().saturating_sub(1));
    let snapshot = snapshot_sheet(&loaded.sheets[active]);

    *state.wb.lock().unwrap() = loaded;
    Ok(Some(OpenResult { path: path_str, snapshot }))
}

/// Where `save_workbook` wrote.
#[derive(Serialize)]
struct SaveResult {
    path: String,
}

/// Save the live workbook in `format` (csv / xlsx / ods / json). If `path` is
/// given and already matches the format, write straight there; otherwise show a
/// native save dialog. `Ok(None)` when the user cancels.
#[tauri::command]
fn save_workbook(
    app: AppHandle,
    state: State<AppState>,
    format: String,
    path: Option<String>,
) -> Result<Option<SaveResult>, String> {
    let fmt = match format.as_str() {
        "csv" => Format::Csv,
        "xlsx" => Format::Xlsx,
        "ods" => Format::Ods,
        "json" => Format::Json,
        other => return Err(format!("unknown format: {other}")),
    };

    let target = match path {
        Some(p) if Format::from_path(&p) == Some(fmt) => std::path::PathBuf::from(p),
        _ => {
            let picked = app
                .dialog()
                .file()
                .set_file_name(format!("Untitled.{}", fmt.extension()))
                .add_filter(fmt.extension().to_uppercase(), &[fmt.extension()])
                .blocking_save_file();
            let Some(file) = picked else {
                return Ok(None);
            };
            file.into_path().map_err(|e| e.to_string())?
        }
    };

    let path_str = target.to_string_lossy().into_owned();
    {
        let wb = state.wb.lock().unwrap();
        sheet_io::save(&wb, &path_str, fmt)?;
    }
    Ok(Some(SaveResult { path: path_str }))
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState { wb: Mutex::new(Workbook::new()) })
        .invoke_handler(tauri::generate_handler![
            new_workbook,
            set_cell,
            open_workbook,
            save_workbook
        ])
        .run(tauri::generate_context!())
        .expect("error while running lattice");
}
