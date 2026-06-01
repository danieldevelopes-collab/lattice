//! Cell styles (font, fill, borders-to-come, alignment) with interning.
//!
//! A spreadsheet has far more cells than distinct appearances: a whole column
//! is usually one style, a header row another. Storing a full [`Style`] on every
//! cell would waste memory and make equality checks slow. Instead a cell holds a
//! small [`StyleId`] that points into a [`StyleTable`], and the table guarantees
//! that two equal styles share one id. Id `0` is always the default style, so a
//! freshly created cell needs no explicit style at all.

use serde::{Deserialize, Serialize};

/// Horizontal text alignment within a cell. Defaults to [`HAlign::Left`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum HAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// Vertical text alignment within a cell. Defaults to [`VAlign::Bottom`], which
/// matches a typical spreadsheet's out-of-the-box behaviour.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VAlign {
    Top,
    Middle,
    #[default]
    Bottom,
}

/// The full visual description of a cell: its font weight/slant/decoration,
/// optional font family, point size, text colour and fill colour (as opaque
/// strings the renderer interprets, e.g. `"#FF0000"`), alignment and wrapping.
///
/// `Default` is the plain, unstyled cell, so `Style::default()` is what id `0`
/// in a [`StyleTable`] always denotes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub font: Option<String>,
    pub size: Option<u16>,
    pub color: Option<String>,
    pub fill: Option<String>,
    pub align: HAlign,
    pub valign: VAlign,
    pub wrap: bool,
}

/// A compact handle into a [`StyleTable`]. `StyleId(0)` is, by construction, the
/// default style; cells default to it (and `u32::default()` is `0`).
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
pub struct StyleId(pub u32);

/// An interning pool of [`Style`]s. Equal styles collapse to one id, and the
/// default style is seeded at id `0` by [`StyleTable::new`].
///
/// The pool only ever grows: ids stay valid for the lifetime of the table, so a
/// [`StyleId`] handed out earlier can always be resolved with [`get`].
///
/// Dedup is a linear scan over the stored styles, compared by [`PartialEq`]. A
/// sheet accumulates only a handful of distinct styles in practice, so this
/// stays cheap while keeping [`Style`] free of the `Hash`/`Eq` bounds a hashed
/// index would demand (its `Option<String>` colours compare structurally).
///
/// [`get`]: StyleTable::get
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StyleTable {
    /// The distinct styles, indexed by their id.
    styles: Vec<Style>,
}

impl StyleTable {
    /// Create a table containing only the default style at id `0`.
    pub fn new() -> Self {
        StyleTable {
            styles: vec![Style::default()],
        }
    }

    /// Return the id for `style`, inserting it if it is not already present.
    /// Equal styles always map to the same id.
    pub fn intern(&mut self, style: &Style) -> StyleId {
        if let Some(pos) = self.styles.iter().position(|s| s == style) {
            return StyleId(pos as u32);
        }
        let id = self.styles.len() as u32;
        self.styles.push(style.clone());
        StyleId(id)
    }

    /// Resolve an id to its style. An id this table never handed out falls back
    /// to the default style rather than panicking, so a stale or corrupted id is
    /// harmless to render.
    pub fn get(&self, id: StyleId) -> &Style {
        self.styles.get(id.0 as usize).unwrap_or(&self.styles[0])
    }

    /// The number of distinct styles held (always at least 1 for the default).
    pub fn len(&self) -> usize {
        self.styles.len()
    }

    /// Whether the table holds only the seeded default style.
    pub fn is_empty(&self) -> bool {
        self.styles.len() <= 1
    }
}

impl Default for StyleTable {
    fn default() -> Self {
        StyleTable::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_defaults() {
        assert_eq!(HAlign::default(), HAlign::Left);
        assert_eq!(VAlign::default(), VAlign::Bottom);
    }

    #[test]
    fn default_style_is_seeded_at_zero() {
        let table = StyleTable::new();
        assert_eq!(table.len(), 1);
        assert!(table.is_empty());
        assert_eq!(table.get(StyleId(0)), &Style::default());
        assert_eq!(StyleId::default(), StyleId(0));
    }

    #[test]
    fn interning_dedups_equal_styles() {
        let mut table = StyleTable::new();
        let bold = Style { bold: true, ..Style::default() };

        let a = table.intern(&bold);
        let b = table.intern(&bold);
        assert_eq!(a, b, "equal styles must share an id");
        assert_eq!(table.len(), 2, "only one new style stored");

        // Interning the default returns id 0 without growing the table.
        let d = table.intern(&Style::default());
        assert_eq!(d, StyleId(0));
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn distinct_styles_get_distinct_ids() {
        let mut table = StyleTable::new();
        let bold = Style { bold: true, ..Style::default() };
        let italic = Style { italic: true, ..Style::default() };

        let a = table.intern(&bold);
        let b = table.intern(&italic);
        assert_ne!(a, b);
        assert_eq!(table.len(), 3);
    }

    #[test]
    fn get_round_trips_an_interned_style() {
        let mut table = StyleTable::new();
        let style = Style {
            bold: true,
            italic: true,
            underline: true,
            strike: false,
            font: Some("Inter".to_string()),
            size: Some(14),
            color: Some("#222222".to_string()),
            fill: Some("#FFFFCC".to_string()),
            align: HAlign::Center,
            valign: VAlign::Middle,
            wrap: true,
        };

        let id = table.intern(&style);
        assert_eq!(table.get(id), &style);
        // Re-interning the same content still yields the same id.
        assert_eq!(table.intern(&style), id);
    }

    #[test]
    fn unknown_id_falls_back_to_default() {
        let table = StyleTable::new();
        assert_eq!(table.get(StyleId(999)), &Style::default());
    }

    #[test]
    fn types_are_serde_capable() {
        // A compile-time check that the public types derive serde (the crate
        // has no concrete serializer like serde_json in its deps, so we assert
        // the bounds rather than performing a round-trip here).
        fn assert_serde<T: serde::Serialize + serde::de::DeserializeOwned>() {}
        assert_serde::<Style>();
        assert_serde::<StyleId>();
        assert_serde::<StyleTable>();
        assert_serde::<HAlign>();
        assert_serde::<VAlign>();
    }
}
