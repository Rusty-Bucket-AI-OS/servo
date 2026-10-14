/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! The list of fonts on WASI: the font files under the folders in `RB_FONT_DIRS` (default `/fonts`), read once. A family
//! is the font's name table family (the typographic one if it has one); a face is one font of a file or collection.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use fonts_traits::{FontIdentifier, FontTemplate, FontTemplateDescriptor, LocalFontIdentifier};
use read_fonts::{FileRef, TableProvider};
use skrifa::MetadataProvider;
use skrifa::string::StringId;
use style::Atom;
use style::values::computed::font::GenericFontFamily;
use style::values::computed::{FontStyle, FontWeight, FontWidth};

use crate::{FallbackFontSelectionOptions, LowercaseFontFamilyName};

struct Face {
    path: String,
    index: u32,
    descriptor: FontTemplateDescriptor,
}

/// The families by lower-case name (with the name as the font spells it), found once.
static INDEX: LazyLock<HashMap<String, (String, Vec<Face>)>> = LazyLock::new(build_index);

/// Whole font files read once and shared, so two faces of a collection or two sizes of a font read the file once.
static FILES: LazyLock<Mutex<HashMap<String, Arc<Vec<u8>>>>> = LazyLock::new(Default::default);

pub(crate) fn file_bytes(path: &str) -> Option<Arc<Vec<u8>>> {
    let mut files = FILES.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(b) = files.get(path) {
        return Some(b.clone());
    }
    let bytes = Arc::new(std::fs::read(path).ok()?);
    files.insert(path.to_owned(), bytes.clone());
    Some(bytes)
}

fn font_dirs() -> Vec<PathBuf> {
    let dirs = std::env::var("RB_FONT_DIRS").unwrap_or_else(|_| "/fonts".to_owned());
    dirs.split(':').filter(|d| !d.is_empty()).map(PathBuf::from).collect()
}

fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth < 4 {
                walk(&path, depth + 1, out);
            }
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc" | "otc"))
        {
            out.push(path);
        }
    }
}

fn descriptor_of(font: &read_fonts::FontRef<'_>) -> FontTemplateDescriptor {
    let Ok(os2) = font.os2() else { return FontTemplateDescriptor::default() };
    let style = if os2.fs_selection().contains(read_fonts::tables::os2::SelectionFlags::ITALIC) {
        FontStyle::ITALIC
    } else {
        FontStyle::NORMAL
    };
    let width = match os2.us_width_class() {
        1 => FontWidth::ULTRA_CONDENSED,
        2 => FontWidth::EXTRA_CONDENSED,
        3 => FontWidth::CONDENSED,
        4 => FontWidth::SEMI_CONDENSED,
        6 => FontWidth::SEMI_EXPANDED,
        7 => FontWidth::EXPANDED,
        8 => FontWidth::EXTRA_EXPANDED,
        9 => FontWidth::ULTRA_EXPANDED,
        _ => FontWidth::NORMAL,
    };
    FontTemplateDescriptor::new(FontWeight::from_float(os2.us_weight_class() as f32), width, style)
}

fn build_index() -> HashMap<String, (String, Vec<Face>)> {
    let mut paths = Vec::new();
    for dir in font_dirs() {
        walk(&dir, 0, &mut paths);
    }
    paths.sort();
    let mut index: HashMap<String, (String, Vec<Face>)> = HashMap::new();
    for path in paths {
        let Some(path_str) = path.to_str().map(str::to_owned) else { continue };
        let Some(bytes) = file_bytes(&path_str) else { continue };
        let Ok(file) = FileRef::new(&bytes) else { continue };
        for (i, font) in file.fonts().enumerate() {
            let Ok(font) = font else { continue };
            let family = font
                .localized_strings(StringId::TYPOGRAPHIC_FAMILY_NAME)
                .english_or_first()
                .or_else(|| font.localized_strings(StringId::FAMILY_NAME).english_or_first())
                .map(|s| s.to_string());
            let Some(family) = family else { continue };
            let face = Face { path: path_str.clone(), index: i as u32, descriptor: descriptor_of(&font) };
            index.entry(family.to_lowercase()).or_insert_with(|| (family, Vec::new())).1.push(face);
        }
    }
    index
}

pub(crate) fn for_each_available_family<F: FnMut(&str)>(mut callback: F) {
    for (name, _) in INDEX.values() {
        callback(name);
    }
}

pub(crate) fn for_each_variation<F: FnMut(FontTemplate)>(family_name: &str, mut callback: F) {
    let Some((_, faces)) = INDEX.get(&family_name.to_lowercase()) else { return };
    for face in faces {
        let id = LocalFontIdentifier { path: Atom::from(face.path.as_str()), face_index: face.index as u16 };
        callback(FontTemplate::new(FontIdentifier::Local(id), face.descriptor.clone(), None));
    }
}

pub fn fallback_font_families(_options: FallbackFontSelectionOptions) -> Vec<&'static str> {
    vec!["Liberation Sans", "DejaVu Sans", "Noto Sans"]
}

pub(crate) fn default_system_generic_font_family(generic: GenericFontFamily) -> LowercaseFontFamilyName {
    let name = match generic {
        GenericFontFamily::Serif | GenericFontFamily::Cursive | GenericFontFamily::Fantasy => "liberation serif",
        GenericFontFamily::Monospace => "liberation mono",
        _ => "liberation sans",
    };
    // If the preferred family is not installed, any installed one is better than none.
    if INDEX.contains_key(name) {
        return LowercaseFontFamilyName::from(name);
    }
    LowercaseFontFamilyName::from(INDEX.keys().min().map_or(name, String::as_str))
}
