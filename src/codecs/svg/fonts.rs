//! Font loading and fallback strategy for SVG rendering.
//!
//! System fonts are loaded once per process, resolved into fallback faces
//! (a serif default and a CJK-capable face) and shared across renders.
//! Fonts missing from the SVG are substituted with system fonts while
//! emitting a warning, and a text span is never dropped for the lack of a
//! font: as a last resort the request falls back to whatever face the
//! system has.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock};

use resvg::usvg;
use resvg::usvg::fontdb;
use skrifa::MetadataProvider;

/// Seed families for the serif default, tried in order until one exists on
/// the system.
const SERIF_SEED_FAMILIES: &[&str] = &[
    "Times New Roman",  // Windows and macOS
    "Liberation Serif", // Linux, metric-compatible with Times New Roman
    "DejaVu Serif",     // Linux
    "Tinos",            // metric-compatible with Times New Roman
    "Nimbus Roman",     // older Linux distributions
];

/// Seed families for the CJK fallback, tried in order while resolving.
const CJK_SEED_FAMILIES: &[&str] = &[
    "Microsoft YaHei UI", // Windows
    "Microsoft YaHei",    // Windows
    "PingFang SC",        // macOS
    "Hiragino Sans GB",   // macOS
    "Noto Sans SC",       // Linux
    "Noto Sans CJK SC",   // Linux
    "Source Han Sans",    // Linux
    "Source Han Sans SC", // Linux
    "Noto Sans CJK TC",
    "Noto Sans CJK JP",
    "WenQuanYi Zen Hei",
    "Noto Sans SC Variable",
    "Source Han Sans CN",
    "Source Han Sans CN VF",
    "Source Han Sans SC VF",
    "MiSans",
    "MiSans L3",
    "HarmonyOS Sans SC",
    "HONOR Sans CN",
    "OPPO Sans 4.0",
    "vivo Sans SC L3",
    "vivo Sans",
    "vivo Sans SC VF",
    "Droid Sans Fallback",
];

/// Characters probed while scanning for a CJK-capable face: Han, Hiragana
/// and Hangul representatives.
const CJK_PROBE_CHARS: &[char] = &['中', 'あ', '가'];

/// System fonts with the fallback faces resolved against them.
struct ResolvedFonts {
    database: Arc<fontdb::Database>,
    /// Face covering CJK codepoints, if the system has one.
    cjk_face: Option<fontdb::ID>,
}

/// Creates a font resolver that warns about missing fonts and substitutes
/// system fonts.
pub(super) fn font_resolver() -> usvg::FontResolver<'static> {
    usvg::FontResolver {
        select_font: Box::new(select_font),
        select_fallback: Box::new(select_fallback),
    }
}

/// Loads system fonts once, resolves the fallback faces and shares the
/// result across all renders.
fn resolved_fonts() -> &'static ResolvedFonts {
    static FONTS: OnceLock<ResolvedFonts> = OnceLock::new();

    FONTS.get_or_init(|| {
        let mut database = fontdb::Database::new();
        database.load_system_fonts();

        let serif_family = resolve_serif_family(&database);
        if let Some(family) = &serif_family {
            // Point the `serif` generic alias at a family that actually
            // exists, so `font-family="serif"` resolves on every platform.
            database.set_serif_family(family);
        }

        let cjk_face = resolve_cjk_face(&database);

        if database.is_empty() {
            log::error!("No system fonts found, text in SVG images will not be rendered");
        } else {
            if serif_family.is_none() {
                log::warn!(
                    "None of the default serif fonts was found, SVG text may be rendered with an arbitrary font"
                );
            }
            if cjk_face.is_none() {
                log::warn!(
                    "No CJK font was found, CJK characters will not be rendered unless the SVG names an available font"
                );
            }
        }

        log::debug!("loaded {} font faces from the system", database.len());

        ResolvedFonts {
            database: Arc::new(database),
            cjk_face,
        }
    })
}

/// Shares the resolved database with usvg and resvg.
pub(super) fn system_fontdb() -> Arc<fontdb::Database> {
    resolved_fonts().database.clone()
}

fn select_font(font: &usvg::Font, fontdb: &mut Arc<fontdb::Database>) -> Option<fontdb::ID> {
    let families: Vec<fontdb::Family> = font
        .families()
        .iter()
        .map(|family| match family {
            usvg::FontFamily::Serif => fontdb::Family::Serif,
            usvg::FontFamily::SansSerif => fontdb::Family::SansSerif,
            usvg::FontFamily::Cursive => fontdb::Family::Cursive,
            usvg::FontFamily::Fantasy => fontdb::Family::Fantasy,
            usvg::FontFamily::Monospace => fontdb::Family::Monospace,
            usvg::FontFamily::Named(name) => fontdb::Family::Name(name.as_str()),
        })
        .collect();

    let query = fontdb::Query {
        families: &families,
        weight: fontdb::Weight(font.weight()),
        stretch: fontdb::Stretch::from(font.stretch()),
        style: fontdb::Style::from(font.style()),
    };

    if let Some(id) = fontdb.query(&query) {
        return Some(id);
    }

    log::warn!(
        "No match for '{}' font-family, substituting the default font",
        font.families()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    );

    // Seed families known to exist per platform, then the serif alias that
    // load time pointed at a family that actually exists.
    let mut fallback: Vec<fontdb::Family> = SERIF_SEED_FAMILIES
        .iter()
        .map(|name| fontdb::Family::Name(name))
        .collect();
    fallback.push(fontdb::Family::Serif);

    if let Some(id) = fontdb.query(&fontdb::Query {
        families: &fallback,
        ..query
    }) {
        return Some(id);
    }

    // Never drop a text span for the lack of a font: render it with
    // whatever face the system has instead.
    fontdb.faces().next().map(|face| face.id)
}

fn select_fallback(
    c: char,
    exclude_fonts: &[fontdb::ID],
    fontdb: &mut Arc<fontdb::Database>,
) -> Option<fontdb::ID> {
    if is_cjk(c) {
        let resolved = resolved_fonts()
            .cjk_face
            .filter(|id| fontdb.face(*id).is_some() && !exclude_fonts.contains(id))
            .filter(|id| has_char(fontdb, *id, c));

        if let Some(id) = resolved {
            let family = fontdb
                .face(id)
                .and_then(|face| {
                    face.families
                        .iter()
                        .find(|(_, language)| *language == fontdb::Language::English_UnitedStates)
                        .or_else(|| face.families.first())
                })
                .map(|(name, _)| name.clone())
                .unwrap_or_else(|| "unknown".into());

            log_missing_glyph(c, &family);
            return Some(id);
        }
    }

    (usvg::FontResolver::default_fallback_selector())(c, exclude_fonts, fontdb)
}

/// Picks the first serif seed family present on the system.
fn resolve_serif_family(database: &fontdb::Database) -> Option<String> {
    SERIF_SEED_FAMILIES
        .iter()
        .find(|name| {
            database
                .query(&fontdb::Query {
                    families: &[fontdb::Family::Name(name)],
                    ..fontdb::Query::default()
                })
                .is_some()
        })
        .map(|name| (*name).to_string())
}

/// Picks a CJK-capable face: seed families first, then a scan over every
/// loaded face for any of the probe characters.
fn resolve_cjk_face(database: &fontdb::Database) -> Option<fontdb::ID> {
    for name in CJK_SEED_FAMILIES {
        let id = database.query(&fontdb::Query {
            families: &[fontdb::Family::Name(name)],
            ..fontdb::Query::default()
        });

        if let Some(id) =
            id.filter(|id| CJK_PROBE_CHARS.iter().any(|&c| has_char(database, *id, c)))
        {
            return Some(id);
        }
    }

    database
        .faces()
        .map(|face| face.id)
        .find(|id| CJK_PROBE_CHARS.iter().any(|&c| has_char(database, *id, c)))
}

/// Warns about a substituted glyph once per character to avoid log flooding
/// on long CJK texts, further occurrences are logged at debug level.
fn log_missing_glyph(c: char, family: &str) {
    static WARNED: OnceLock<Mutex<HashSet<u32>>> = OnceLock::new();

    let warned = WARNED.get_or_init(|| Mutex::new(HashSet::new()));
    let first_time = warned
        .lock()
        .map(|mut warned| warned.insert(u32::from(c)))
        .unwrap_or(true);

    let message = format!(
        "No glyph for U+{:04X} in the selected fonts, substituted '{family}'",
        u32::from(c)
    );

    if first_time {
        log::warn!("{message}");
    } else {
        log::debug!("{message}");
    }
}

/// Checks whether the face supports the character.
///
/// Maps through the character map and rejects glyphs resolved to `.notdef`,
/// mirroring the character check used inside `usvg` which is not exposed
/// publicly.
fn has_char(fontdb: &fontdb::Database, id: fontdb::ID, c: char) -> bool {
    fontdb
        .with_face_data(id, |font_data, face_index| {
            skrifa::FontRef::from_index(font_data, face_index)
                .ok()?
                .charmap()
                .map(c)
        })
        .is_some_and(|glyph| glyph.is_some())
}

/// Checks whether the character belongs to a CJK script (Han, Kana, Hangul
/// and the CJK-specific punctuation and symbol blocks).
fn is_cjk(c: char) -> bool {
    matches!(u32::from(c),
        0x2E80..=0x2EFF      // CJK Radicals Supplement
        | 0x2F00..=0x2FDF    // Kangxi Radicals
        | 0x3000..=0x303F    // CJK Symbols and Punctuation
        | 0x3040..=0x30FF    // Hiragana and Katakana
        | 0x3130..=0x318F    // Hangul Compatibility Jamo
        | 0x3400..=0x4DBF    // CJK Unified Ideographs Extension A
        | 0x4E00..=0x9FFF    // CJK Unified Ideographs
        | 0xAC00..=0xD7AF    // Hangul Syllables
        | 0xF900..=0xFAFF    // CJK Compatibility Ideographs
        | 0xFE30..=0xFE4F    // CJK Compatibility Forms
        | 0xFF00..=0xFFEF    // Halfwidth and Fullwidth Forms
        | 0x20000..=0x2FA1F  // CJK Unified Ideographs Extensions B..F and Supplement
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cjk_ranges() {
        assert!(is_cjk('中'));
        assert!(is_cjk('あ'));
        assert!(is_cjk('한'));
        assert!(is_cjk('，'));
        assert!(is_cjk('Ａ')); // fullwidth latin
        assert!(!is_cjk('A'));
        assert!(!is_cjk('é'));
        assert!(!is_cjk('😀'));
    }

    #[test]
    fn serif_resolution_returns_a_seed() {
        let mut database = fontdb::Database::new();
        database.load_system_fonts();

        if let Some(family) = resolve_serif_family(&database) {
            assert!(SERIF_SEED_FAMILIES.contains(&family.as_str()));

            // the alias must resolve once it points at the found family
            database.set_serif_family(&family);
            assert!(
                database
                    .query(&fontdb::Query {
                        families: &[fontdb::Family::Serif],
                        ..fontdb::Query::default()
                    })
                    .is_some()
            );
        }
    }

    #[test]
    fn cjk_resolution_finds_a_covering_face() {
        let mut database = fontdb::Database::new();
        database.load_system_fonts();

        if let Some(id) = resolve_cjk_face(&database) {
            assert!(CJK_PROBE_CHARS.iter().any(|&c| has_char(&database, id, c)));
        }
    }
}
