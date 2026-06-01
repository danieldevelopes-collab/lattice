//! # sheet-io
//!
//! Persistence and interop for a [`sheet_core::Workbook`]. It turns a workbook
//! into bytes (and back) in four formats:
//!
//! * **JSON** — the native format, lossless: every sheet's cell *inputs* are
//!   stored and replayed, so formulas and literals round-trip exactly.
//! * **CSV** — the active sheet's used range as displayed text; importing a CSV
//!   yields a one-sheet workbook of literal values.
//! * **XLSX** — written with `rust_xlsxwriter`, read with `calamine`.
//! * **ODS** — an OpenDocument Spreadsheet written by hand (a small zip), read
//!   with `calamine`.
//!
//! Exporting always uses the value a cell *already holds* — recalculation is the
//! caller's responsibility, never this crate's.

mod csv_io;
mod json;
mod ods;
mod xlsx;

use sheet_core::Workbook;

/// A file format `sheet-io` can read and write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Csv,
    Xlsx,
    Ods,
    Json,
}

impl Format {
    /// Guess the format from a path's extension (case-insensitive).
    /// `None` when the extension is missing or unrecognised.
    pub fn from_path(path: &str) -> Option<Format> {
        let ext = path.rsplit('.').next()?;
        // If there was no '.', rsplit yields the whole string; guard against that.
        if ext == path && !path.contains('.') {
            return None;
        }
        match ext.to_ascii_lowercase().as_str() {
            "csv" => Some(Format::Csv),
            "xlsx" | "xlsm" => Some(Format::Xlsx),
            "ods" => Some(Format::Ods),
            "json" => Some(Format::Json),
            _ => None,
        }
    }

    /// The canonical file extension (without the dot).
    pub fn extension(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Xlsx => "xlsx",
            Format::Ods => "ods",
            Format::Json => "json",
        }
    }
}

/// Serialise a workbook to bytes in the given format.
pub fn to_bytes(wb: &Workbook, fmt: Format) -> Result<Vec<u8>, String> {
    match fmt {
        Format::Json => json::to_bytes(wb),
        Format::Csv => csv_io::to_bytes(wb),
        Format::Xlsx => xlsx::to_bytes(wb),
        Format::Ods => ods::to_bytes(wb),
    }
}

/// Reconstruct a workbook from bytes in the given format.
pub fn from_bytes(data: &[u8], fmt: Format) -> Result<Workbook, String> {
    match fmt {
        Format::Json => json::from_bytes(data),
        Format::Csv => csv_io::from_bytes(data),
        Format::Xlsx => xlsx::from_bytes(data),
        Format::Ods => ods::from_bytes(data),
    }
}

/// Serialise a workbook and write it to `path`.
pub fn save(wb: &Workbook, path: &str, fmt: Format) -> Result<(), String> {
    let bytes = to_bytes(wb, fmt)?;
    std::fs::write(path, bytes).map_err(|e| format!("writing {path}: {e}"))
}

/// Load a workbook from `path`, picking the format from its extension.
pub fn load(path: &str) -> Result<Workbook, String> {
    let fmt = Format::from_path(path)
        .ok_or_else(|| format!("cannot determine a format from path {path:?}"))?;
    let data = std::fs::read(path).map_err(|e| format!("reading {path}: {e}"))?;
    from_bytes(&data, fmt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sheet_core::{CellRef, Value};

    /// A small workbook of literals (no formula recalc needed) plus one formula
    /// input, exercising numbers, text and booleans across two sheets.
    fn sample() -> Workbook {
        let mut wb = Workbook::new();
        {
            let s = wb.active_sheet_mut();
            s.set_a1("A1", "42");
            s.set_a1("B1", "hello");
            s.set_a1("C1", "TRUE");
            s.set_a1("A2", "3.5");
            s.set_a1("B2", "world");
        }
        let i = wb.add_sheet("Second");
        wb.sheet_mut(i).unwrap().set_a1("A1", "100");
        wb.sheet_mut(i).unwrap().set_a1("A2", "=A1+1");
        wb
    }

    fn cellval(wb: &Workbook, sheet: usize, a1: &str) -> Value {
        wb.sheet(sheet).unwrap().value(CellRef::parse(a1).unwrap())
    }

    fn input(wb: &Workbook, sheet: usize, a1: &str) -> String {
        wb.sheet(sheet)
            .unwrap()
            .get(CellRef::parse(a1).unwrap())
            .map(|c| c.input.clone())
            .unwrap_or_default()
    }

    #[test]
    fn format_from_path_and_extension() {
        assert_eq!(Format::from_path("book.csv"), Some(Format::Csv));
        assert_eq!(Format::from_path("/tmp/a.XLSX"), Some(Format::Xlsx));
        assert_eq!(Format::from_path("data.ods"), Some(Format::Ods));
        assert_eq!(Format::from_path("save.json"), Some(Format::Json));
        assert_eq!(Format::from_path("noext"), None);
        assert_eq!(Format::from_path("weird.txt"), None);
        assert_eq!(Format::Csv.extension(), "csv");
        assert_eq!(Format::Xlsx.extension(), "xlsx");
        assert_eq!(Format::Ods.extension(), "ods");
        assert_eq!(Format::Json.extension(), "json");
    }

    #[test]
    fn json_round_trips_inputs_and_values_exactly() {
        let wb = sample();
        let bytes = to_bytes(&wb, Format::Json).unwrap();
        let back = from_bytes(&bytes, Format::Json).unwrap();

        assert_eq!(back.sheets.len(), 2);
        assert_eq!(back.sheets[0].name, "Sheet1");
        assert_eq!(back.sheets[1].name, "Second");
        assert_eq!(back.active, wb.active);

        // Inputs preserved verbatim, including the formula text on sheet 2.
        assert_eq!(input(&back, 0, "A1"), "42");
        assert_eq!(input(&back, 0, "B1"), "hello");
        assert_eq!(input(&back, 0, "C1"), "TRUE");
        assert_eq!(input(&back, 1, "A2"), "=A1+1");

        // Literal values are recomputed on load and match.
        assert_eq!(cellval(&back, 0, "A1"), Value::Number(42.0));
        assert_eq!(cellval(&back, 0, "B1"), Value::Text("hello".into()));
        assert_eq!(cellval(&back, 0, "C1"), Value::Bool(true));
        assert_eq!(cellval(&back, 0, "A2"), Value::Number(3.5));
        // The formula cell is recognised as a formula again.
        assert!(back.sheets[1]
            .get(CellRef::parse("A2").unwrap())
            .unwrap()
            .is_formula());
    }

    #[test]
    fn csv_round_trips_the_active_sheet() {
        let wb = sample();
        let bytes = to_bytes(&wb, Format::Csv).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        // Displayed values of the active sheet's used range (A1:C2).
        assert!(text.contains("42"));
        assert!(text.contains("hello"));
        assert!(text.contains("TRUE"));
        assert!(text.contains("world"));

        let back = from_bytes(&bytes, Format::Csv).unwrap();
        assert_eq!(back.sheets.len(), 1);
        // Round-trips as literal inputs.
        assert_eq!(input(&back, 0, "A1"), "42");
        assert_eq!(input(&back, 0, "B1"), "hello");
        assert_eq!(cellval(&back, 0, "A1"), Value::Number(42.0));
        assert_eq!(cellval(&back, 0, "C1"), Value::Bool(true));
        assert_eq!(cellval(&back, 0, "B2"), Value::Text("world".into()));
        // The gap left by the empty A-column below row 2 is not invented.
        assert_eq!(cellval(&back, 0, "A2"), Value::Number(3.5));
    }

    #[test]
    fn xlsx_starts_with_pk_magic() {
        let wb = sample();
        let bytes = to_bytes(&wb, Format::Xlsx).unwrap();
        assert!(bytes.len() > 2);
        assert_eq!(&bytes[0..2], b"PK", "xlsx is a zip, must start with PK");
    }

    #[test]
    fn xlsx_reads_back_through_calamine() {
        let wb = sample();
        let bytes = to_bytes(&wb, Format::Xlsx).unwrap();
        let back = from_bytes(&bytes, Format::Xlsx).unwrap();
        assert_eq!(back.sheets.len(), 2);
        // Values survive the xlsx round-trip (literals computed by the engine).
        assert_eq!(cellval(&back, 0, "A1"), Value::Number(42.0));
        assert_eq!(cellval(&back, 0, "B1"), Value::Text("hello".into()));
        assert_eq!(cellval(&back, 0, "C1"), Value::Bool(true));
        assert_eq!(cellval(&back, 0, "A2"), Value::Number(3.5));
        assert_eq!(back.sheets[1].name, "Second");
        assert_eq!(cellval(&back, 1, "A1"), Value::Number(100.0));
    }

    #[test]
    fn ods_is_a_valid_opendocument_zip() {
        let wb = sample();
        let bytes = to_bytes(&wb, Format::Ods).unwrap();
        assert_eq!(&bytes[0..2], b"PK", "ods is a zip, must start with PK");
        // The mimetype must appear (stored uncompressed) in the archive.
        let haystack = String::from_utf8_lossy(&bytes);
        assert!(
            haystack.contains("application/vnd.oasis.opendocument.spreadsheet"),
            "ods must declare its OpenDocument mimetype"
        );
    }

    #[test]
    fn ods_reads_back_through_calamine() {
        let wb = sample();
        let bytes = to_bytes(&wb, Format::Ods).unwrap();
        let back = from_bytes(&bytes, Format::Ods).unwrap();
        assert_eq!(back.sheets.len(), 2);
        assert_eq!(cellval(&back, 0, "A1"), Value::Number(42.0));
        assert_eq!(cellval(&back, 0, "B1"), Value::Text("hello".into()));
        assert_eq!(cellval(&back, 0, "A2"), Value::Number(3.5));
        assert_eq!(back.sheets[1].name, "Second");
        assert_eq!(cellval(&back, 1, "A1"), Value::Number(100.0));
    }

    #[test]
    fn save_and_load_via_a_tempfile() {
        let wb = sample();
        let dir = std::env::temp_dir();
        let path = dir
            .join(format!("sheet_io_test_{}.json", std::process::id()))
            .to_string_lossy()
            .into_owned();
        save(&wb, &path, Format::Json).unwrap();
        let back = load(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back.sheets.len(), 2);
        assert_eq!(input(&back, 1, "A2"), "=A1+1");
    }
}
