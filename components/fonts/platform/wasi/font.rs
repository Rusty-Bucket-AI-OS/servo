/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use app_units::Au;
use euclid::default::{Point2D, Rect, Size2D};
use fonts_traits::{FontIdentifier, FontTemplateDescriptor, LocalFontIdentifier};
use read_fonts::types::Tag;
use read_fonts::{FontRef, ReadError, TableProvider};
use skrifa::instance::{LocationRef, Size};
use skrifa::{GlyphId as SkrifaGlyphId, MetadataProvider};
use std::sync::Arc;
use style::Zero;
use webrender_api::{FontInstanceFlags, FontVariation};

use crate::FontData;
use crate::font::{FontMetrics, FontTableMethods, FractionalPixel, PlatformFontMethods};
use crate::glyph::GlyphId;

const SEMI_BOLD: u16 = 600;

/// Where a font's bytes live: a web font Servo downloaded, or a file read once into memory.
#[derive(Clone)]
enum Backing {
    Web(FontData),
    Local(Arc<Vec<u8>>, u32),
}

impl Backing {
    fn font_ref(&self) -> Result<FontRef<'_>, ReadError> {
        match self {
            Backing::Web(data) => FontRef::new(data.as_ref()),
            Backing::Local(bytes, index) => FontRef::from_index(bytes, *index),
        }
    }
}

impl std::fmt::Debug for Backing {
    fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Ok(())
    }
}

#[derive(Debug)]
pub struct FontTable {
    backing: Backing,
    tag: Tag,
}

impl FontTableMethods for FontTable {
    fn buffer(&self) -> &[u8] {
        self.backing
            .font_ref()
            .ok()
            .and_then(|f| f.table_data(self.tag))
            .map(|d| d.as_bytes())
            .unwrap_or(&[])
    }
}

#[derive(Debug)]
pub struct PlatformFont {
    backing: Backing,
    size: Au,
    variations: Vec<FontVariation>,
    synthetic_bold: bool,
}

fn tag(v: &FontVariation) -> Tag {
    Tag::from_be_bytes(v.tag.to_be_bytes())
}

impl PlatformFont {
    fn new(backing: Backing, size: Option<Au>, synthetic_bold: bool) -> Result<PlatformFont, &'static str> {
        let font = backing.font_ref().map_err(|_| "Could not read the font")?;
        let is_bold = font.os2().is_ok_and(|t| t.us_weight_class() >= SEMI_BOLD);
        let is_variable = font.fvar().is_ok_and(|t| t.axis_count() > 0);
        // A font that is already bold, or variable, is not emboldened.
        let synthetic_bold = !is_bold && !is_variable && synthetic_bold;
        Ok(PlatformFont { backing, size: size.unwrap_or_else(Au::zero), variations: vec![], synthetic_bold })
    }

    /// Runs `f` with the parsed font, the size and the variation location (all that skrifa needs), or `None` if unreadable.
    fn with<R>(&self, f: impl FnOnce(&FontRef<'_>, Size, LocationRef<'_>) -> R) -> Option<R> {
        let font = self.backing.font_ref().ok()?;
        let location = font.axes().location(self.variations.iter().map(|v| (tag(v), v.value)));
        let px = self.size.to_f32_px();
        let size = if px > 0.0 { Size::new(px) } else { Size::unscaled() };
        Some(f(&font, size, LocationRef::from(&location)))
    }
}

impl PlatformFontMethods for PlatformFont {
    fn new_from_data(
        _font_identifier: FontIdentifier,
        font_data: &FontData,
        requested_size: Option<Au>,
        synthetic_bold: bool,
    ) -> Result<PlatformFont, &'static str> {
        Self::new(Backing::Web(font_data.clone()), requested_size, synthetic_bold)
    }

    fn new_from_local_font_identifier(
        font_identifier: LocalFontIdentifier,
        requested_size: Option<Au>,
        synthetic_bold: bool,
    ) -> Result<PlatformFont, &'static str> {
        let bytes = super::font_list::file_bytes(&font_identifier.path).ok_or("Could not read the font file")?;
        Self::new(Backing::Local(bytes, font_identifier.index()), requested_size, synthetic_bold)
    }

    fn copy_with_variations(mut self, _: &FontIdentifier, variations: &[FontVariation]) -> Result<Self, &'static str> {
        self.variations = variations.to_vec();
        Ok(self)
    }

    fn descriptor(&self) -> FontTemplateDescriptor {
        self.backing
            .font_ref()
            .ok()
            .and_then(|f| f.os2().ok())
            .map(|os2| Self::descriptor_from_os2_table(&os2))
            .unwrap_or_default()
    }

    fn glyph_index(&self, codepoint: char) -> Option<GlyphId> {
        self.with(|f, _, _| f.charmap().map(codepoint).map(|g| g.to_u32() as GlyphId)).flatten()
    }

    fn glyph_h_kerning(&self, _first: GlyphId, _second: GlyphId) -> FractionalPixel {
        // Pair kerning comes from the shaper (HarfBuzz reads GPOS); the legacy `kern` table is not consulted here.
        0.0
    }

    fn glyph_h_advance(&self, glyph: GlyphId) -> Option<FractionalPixel> {
        self.with(|f, size, loc| f.glyph_metrics(size, loc).advance_width(SkrifaGlyphId::new(glyph)).map(f64::from)).flatten()
    }

    fn metrics(&self) -> FontMetrics {
        let m = self
            .with(|f, size, loc| {
                let metrics = f.metrics(size, loc);
                let advance = |c: char| {
                    f.charmap().map(c).and_then(|g| f.glyph_metrics(size, loc).advance_width(g)).map(f64::from)
                };
                (metrics, advance('0'), advance('\u{6C34}'), advance(' '))
            });
        let Some((m, zero, ic, space)) = m else { return empty_metrics() };
        let em_size = if self.size.is_zero() { f64::from(m.units_per_em) } else { self.size.to_f64_px() };
        let ascent = f64::from(m.ascent);
        let descent = -f64::from(m.descent);
        let leading = f64::from(m.leading);
        let line_height = ascent + descent + leading;
        let (underline_size, underline_offset) = m.underline.map_or((em_size / 20.0, -em_size / 10.0), |d| (f64::from(d.thickness), f64::from(d.offset)));
        let (strike_size, strike_offset) = m
            .strikeout
            .map_or((underline_size, em_size * 409.0 / 2048.0 + 0.5 * underline_size), |d| (f64::from(d.thickness), f64::from(d.offset)));
        let x_height = m.x_height.map_or(0.5 * em_size, f64::from);
        let max_advance = m.max_width.map_or(em_size, f64::from);
        let average_advance = m.average_width.map(f64::from).filter(|a| *a > 0.0).or(zero).unwrap_or(max_advance);
        FontMetrics {
            underline_size: Au::from_f64_px(underline_size),
            underline_offset: Au::from_f64_px(underline_offset + 0.5),
            strikeout_size: Au::from_f64_px(strike_size),
            strikeout_offset: Au::from_f64_px(strike_offset),
            leading: Au::from_f64_px(leading),
            x_height: Au::from_f64_px(x_height),
            em_size: Au::from_f64_px(em_size),
            ascent: Au::from_f64_px(ascent),
            descent: Au::from_f64_px(descent),
            max_advance: Au::from_f64_px(max_advance),
            average_advance: Au::from_f64_px(average_advance),
            line_gap: Au::from_f64_px(line_height),
            zero_horizontal_advance: zero.map(Au::from_f64_px),
            ic_horizontal_advance: ic.map(Au::from_f64_px),
            space_advance: Au::from_f64_px(space.unwrap_or(average_advance)),
        }
    }

    fn table_for_tag(&self, tag: Tag) -> Option<FontTable> {
        let font = self.backing.font_ref().ok()?;
        font.table_data(tag)?;
        Some(FontTable { backing: self.backing.clone(), tag })
    }

    fn typographic_bounds(&self, glyph_id: GlyphId) -> Rect<f32> {
        self.with(|f, size, loc| f.glyph_metrics(size, loc).bounds(SkrifaGlyphId::new(glyph_id)))
            .flatten()
            .map(|b| Rect::new(Point2D::new(b.x_min, b.y_min), Size2D::new(b.x_max - b.x_min, b.y_max - b.y_min)))
            .unwrap_or_default()
    }

    fn webrender_font_instance_flags(&self) -> FontInstanceFlags {
        let mut flags = FontInstanceFlags::empty();
        if self.synthetic_bold {
            flags |= FontInstanceFlags::SYNTHETIC_BOLD;
        }
        flags
    }

    fn variations(&self) -> &[FontVariation] {
        &self.variations
    }
}

fn empty_metrics() -> FontMetrics {
    FontMetrics {
        underline_size: Au::zero(),
        underline_offset: Au::zero(),
        strikeout_size: Au::zero(),
        strikeout_offset: Au::zero(),
        leading: Au::zero(),
        x_height: Au::zero(),
        em_size: Au::zero(),
        ascent: Au::zero(),
        descent: Au::zero(),
        max_advance: Au::zero(),
        average_advance: Au::zero(),
        line_gap: Au::zero(),
        zero_horizontal_advance: None,
        ic_horizontal_advance: None,
        space_advance: Au::zero(),
    }
}
