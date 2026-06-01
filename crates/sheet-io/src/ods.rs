//! OpenDocument Spreadsheet (`.ods`) export and import.
//!
//! An ODS file is a zip with a fixed shape. The spec is strict about one thing:
//! the very first entry must be an uncompressed `mimetype` whose bytes are
//! exactly `application/vnd.oasis.opendocument.spreadsheet`, so the format can
//! be sniffed without inflating anything. We then add the `META-INF/manifest.xml`
//! catalogue and a `content.xml` holding the tables. Reading is delegated to
//! `calamine`'s ODS support, which hands back the same typed values as XLSX.

use calamine::{Data, Ods, Reader};
use sheet_core::{CellRef, Value, Workbook};
use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const MIMETYPE: &str = "application/vnd.oasis.opendocument.spreadsheet";

pub fn to_bytes(wb: &Workbook) -> Result<Vec<u8>, String> {
    let buf = Cursor::new(Vec::new());
    let mut zip = ZipWriter::new(buf);

    // 1. mimetype — FIRST and STORED (uncompressed), per the OpenDocument spec.
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    zip.start_file("mimetype", stored).map_err(|e| format!("ods mimetype entry: {e}"))?;
    zip.write_all(MIMETYPE.as_bytes()).map_err(|e| format!("ods mimetype body: {e}"))?;

    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    // 2. The manifest cataloguing the parts.
    zip.start_file("META-INF/manifest.xml", deflated)
        .map_err(|e| format!("ods manifest entry: {e}"))?;
    zip.write_all(manifest_xml().as_bytes()).map_err(|e| format!("ods manifest body: {e}"))?;

    // 3. The actual spreadsheet content.
    zip.start_file("content.xml", deflated).map_err(|e| format!("ods content entry: {e}"))?;
    zip.write_all(content_xml(wb).as_bytes()).map_err(|e| format!("ods content body: {e}"))?;

    let cursor = zip.finish().map_err(|e| format!("ods finish: {e}"))?;
    Ok(cursor.into_inner())
}

fn manifest_xml() -> String {
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    s.push_str("<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.2\">\n");
    s.push_str(&format!(
        "  <manifest:file-entry manifest:full-path=\"/\" manifest:version=\"1.2\" manifest:media-type=\"{MIMETYPE}\"/>\n"
    ));
    s.push_str("  <manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/>\n");
    s.push_str("</manifest:manifest>\n");
    s
}

fn content_xml(wb: &Workbook) -> String {
    // No pretty-printing inside the spreadsheet body: ODS readers (calamine
    // among them) treat any text node between table elements as cell content,
    // so stray indentation whitespace corrupts the parse. We keep the prolog on
    // its own line but emit every table/row/cell tag back-to-back.
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    s.push_str("<office:document-content");
    s.push_str(" xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\"");
    s.push_str(" xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\"");
    s.push_str(" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\"");
    s.push_str(" office:version=\"1.2\">");
    s.push_str("<office:body>");
    s.push_str("<office:spreadsheet>");

    for sheet in &wb.sheets {
        s.push_str(&format!(
            "<table:table table:name=\"{}\">",
            xml_attr(&sheet.name)
        ));

        match sheet.used_bounds() {
            Some(bounds) => {
                for row in 0..=bounds.row {
                    s.push_str("<table:table-row>");
                    for col in 0..=bounds.col {
                        let value = sheet.value(CellRef::new(col, row));
                        s.push_str(&cell_xml(&value));
                    }
                    s.push_str("</table:table-row>");
                }
            }
            None => {
                // An empty sheet still needs a (trivial) row to be well-formed.
                s.push_str("<table:table-row><table:table-cell/></table:table-row>");
            }
        }

        s.push_str("</table:table>");
    }

    s.push_str("</office:spreadsheet>");
    s.push_str("</office:body>");
    s.push_str("</office:document-content>");
    s
}

/// One `<table:table-cell>`, typed the OpenDocument way.
fn cell_xml(value: &Value) -> String {
    match value {
        Value::Empty => "<table:table-cell/>".to_string(),
        Value::Number(n) => format!(
            "<table:table-cell office:value-type=\"float\" office:value=\"{}\"><text:p>{}</text:p></table:table-cell>",
            ods_number(*n),
            xml_text(&sheet_core::format_number(*n)),
        ),
        Value::Bool(b) => format!(
            "<table:table-cell office:value-type=\"boolean\" office:boolean-value=\"{}\"><text:p>{}</text:p></table:table-cell>",
            if *b { "true" } else { "false" },
            if *b { "TRUE" } else { "FALSE" },
        ),
        Value::Text(t) => format!(
            "<table:table-cell office:value-type=\"string\"><text:p>{}</text:p></table:table-cell>",
            xml_text(t),
        ),
        Value::Error(e) => format!(
            "<table:table-cell office:value-type=\"string\"><text:p>{}</text:p></table:table-cell>",
            xml_text(&e.to_string()),
        ),
    }
}

/// Format a float for the `office:value` attribute: a plain machine number, no
/// thousands separators, `.` decimal, no NaN/Inf (clamped to 0 for safety).
fn ods_number(n: f64) -> String {
    if n.is_finite() {
        // Ryu-style default formatting is fine; integers render without ".0".
        let s = format!("{n}");
        s
    } else {
        "0".to_string()
    }
}

/// Escape text node content.
fn xml_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}

/// Escape an attribute value (text rules plus quotes).
fn xml_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

pub fn from_bytes(data: &[u8]) -> Result<Workbook, String> {
    let cursor = Cursor::new(data.to_vec());
    let mut reader: Ods<_> = Ods::new(cursor).map_err(|e| format!("ods open: {e}"))?;

    let mut wb = Workbook::new();
    let sheet_names = reader.sheet_names().to_vec();
    if sheet_names.is_empty() {
        return Ok(wb);
    }

    for (i, name) in sheet_names.iter().enumerate() {
        let range = reader
            .worksheet_range(name)
            .map_err(|e| format!("ods read sheet {name:?}: {e}"))?;

        let idx = if i == 0 {
            wb.sheets[0].name = name.clone();
            0
        } else {
            wb.add_sheet(name.clone())
        };
        let sheet = wb.sheet_mut(idx).expect("just created");

        let Some((base_row, base_col)) = range.start() else {
            continue;
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
