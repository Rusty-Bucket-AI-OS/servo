// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! The glyph rasterizer's platform interface on `skrifa` (outlines) and `tiny-skia` (coverage): no C, no system fonts,
//! so it builds for WebAssembly. Used when the target is WASI, or with the `skrifa_platform` feature on any target (to compare
//! it with FreeType). Grayscale only: outlines unhinted, no color or bitmap glyphs, no synthetic italics, no flips or
//! transposes (WebRender asks for those only for vertical text and for some bitmap fonts). The twelve functions are the whole interface.

use std::collections::HashMap;
use std::sync::Arc;

use api::{FontKey, FontRenderMode, FontVariation, GlyphDimensions, NativeFontHandle};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::types::Tag;
use skrifa::{FontRef, GlyphId, MetadataProvider};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};

use crate::rasterizer::{FontInstance, GlyphKey, GlyphRasterError, GlyphRasterResult, RasterizedGlyph};

struct Face {
    data: Arc<Vec<u8>>,
    index: u32,
}

#[derive(Default)]
pub struct FontContext {
    faces: HashMap<FontKey, Face>,
}

/// An outline in y-down pixel coordinates (one unit is one device pixel) and its box.
struct Shape {
    path: tiny_skia::Path,
}

/// Collects skrifa's y-up outline into a y-down tiny-skia path.
struct Pen(PathBuilder);

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, -y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, -y);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to(cx0, -cy0, x, -y);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.cubic_to(cx0, -cy0, cx1, -cy1, x, -y);
    }
    fn close(&mut self) {
        self.0.close();
    }
}

fn tag(variation: &FontVariation) -> Tag {
    Tag::from_be_bytes(variation.tag.to_be_bytes())
}

impl FontContext {
    pub fn distribute_across_threads() -> bool {
        false
    }

    pub fn new() -> FontContext {
        FontContext::default()
    }

    pub fn add_raw_font(&mut self, font_key: &FontKey, bytes: Arc<Vec<u8>>, index: u32) {
        if FontRef::from_index(&bytes, index).is_ok() {
            self.faces.insert(*font_key, Face { data: bytes, index });
        }
    }

    /// A font Servo found on disk arrives as a path and a face index (there is no system font database here, but there are
    /// files): read it and treat it as bytes.
    pub fn add_native_font(&mut self, font_key: &FontKey, native_font_handle: NativeFontHandle) {
        if let Ok(bytes) = std::fs::read(&native_font_handle.path) {
            self.add_raw_font(font_key, Arc::new(bytes), native_font_handle.index);
        }
    }

    pub fn delete_font(&mut self, font_key: &FontKey) {
        self.faces.remove(font_key);
    }

    pub fn delete_font_instance(&mut self, _instance: &FontInstance) {}

    pub fn get_glyph_index(&mut self, font_key: FontKey, ch: char) -> Option<u32> {
        let face = self.faces.get(&font_key)?;
        let font = FontRef::from_index(&face.data, face.index).ok()?;
        font.charmap().map(ch).map(|g| g.to_u32())
    }

    pub fn get_glyph_dimensions(&mut self, font: &FontInstance, key: &GlyphKey) -> Option<GlyphDimensions> {
        let (shape, advance) = self.shape(font, key)?;
        let (x0, y1, w, h) = pixel_box(&shape, font, key);
        Some(GlyphDimensions { left: x0, top: y1, width: w, height: h, advance })
    }

    pub fn prepare_font(font: &mut FontInstance) {
        // Grayscale only: a request for subpixel (LCD) text is answered with grayscale coverage in all three channels.
        font.render_mode = font.render_mode.limit_by(FontRenderMode::Subpixel);
    }

    pub fn begin_rasterize(_font: &FontInstance) {}

    pub fn end_rasterize(_font: &FontInstance) {}

    pub fn rasterize_glyph(&mut self, font: &FontInstance, key: &GlyphKey) -> GlyphRasterResult {
        let (shape, _) = self.shape(font, key).ok_or(GlyphRasterError::LoadFailed)?;
        let (x0, y1, width, height) = pixel_box(&shape, font, key);
        if width <= 0 || height <= 0 {
            return Err(GlyphRasterError::LoadFailed); // a space, or a glyph with no outline
        }
        let pad = i32::from(font.use_texture_padding());
        let (w, h) = ((width + 2 * pad) as u32, (height + 2 * pad) as u32);
        let mut pixmap = Pixmap::new(w, h).ok_or(GlyphRasterError::LoadFailed)?;
        let mut paint = Paint::default();
        paint.set_color_rgba8(0, 0, 0, 255);
        paint.anti_alias = true;
        let (dx, dy) = font.get_subpx_offset(key);
        // Pixel (0, 0) of the bitmap is the top-left corner of the box (x0 across, y1 above the baseline); the path is shifted
        // by the subpixel offset and then into the box.
        let transform = Transform::from_translate(dx as f32 - x0 as f32 + pad as f32, dy as f32 + y1 as f32 + pad as f32);
        pixmap.fill_path(&shape.path, &paint, FillRule::Winding, transform, None);
        // WebRender wants BGRA with the coverage in every channel, premultiplied.
        let mut bytes = Vec::with_capacity(pixmap.data().len());
        for px in pixmap.data().chunks_exact(4) {
            let a = px[3];
            bytes.extend_from_slice(&[a, a, a, a]);
        }
        let format = match font.render_mode {
            FontRenderMode::Subpixel => font.get_subpixel_glyph_format(),
            _ => font.get_alpha_glyph_format(),
        };
        Ok(RasterizedGlyph {
            left: (x0 - pad) as f32,
            top: (y1 + pad) as f32,
            width: w as i32,
            height: h as i32,
            scale: 1.0,
            format,
            bytes,
            is_packed_glyph: false,
        })
    }

    /// The glyph's outline at the instance's size, and its advance, both in device pixels.
    fn shape(&self, font: &FontInstance, key: &GlyphKey) -> Option<(Shape, f32)> {
        let face = self.faces.get(&font.font_key)?;
        let f = FontRef::from_index(&face.data, face.index).ok()?;
        let (_, y_scale) = font.transform.compute_scale().unwrap_or((1.0, 1.0));
        let size = Size::new((font.size.to_f64_px() * y_scale) as f32);
        let location = f.axes().location(font.variations.iter().map(|v| (tag(v), v.value)));
        let gid = GlyphId::new(key.index());
        let advance = f.glyph_metrics(size, &location).advance_width(gid).unwrap_or(0.0);
        let mut pen = Pen(PathBuilder::new());
        if let Some(outline) = f.outline_glyphs().get(gid) {
            outline.draw(DrawSettings::unhinted(size, LocationRef::from(&location)), &mut pen).ok()?;
        }
        // An empty outline (a space) has dimensions of zero, not an error.
        let path = pen.0.finish().unwrap_or_else(|| PathBuilder::from_rect(tiny_skia::Rect::from_xywh(0.0, 0.0, 0.0001, 0.0001).expect("tiny rect")));
        Some((Shape { path }, advance))
    }
}

/// (left, top, width, height) of the whole-pixel box around the outline once the subpixel offset is applied; `top` is the
/// distance from the baseline to the top row, positive up, as FreeType reports it.
fn pixel_box(shape: &Shape, font: &FontInstance, key: &GlyphKey) -> (i32, i32, i32, i32) {
    let b = shape.path.bounds();
    if b.width() <= 0.001 && b.height() <= 0.001 {
        return (0, 0, 0, 0);
    }
    let (dx, dy) = font.get_subpx_offset(key);
    let (dx, dy) = (dx as f32, dy as f32);
    let x0 = (b.left() + dx).floor() as i32;
    let x1 = (b.right() + dx).ceil() as i32;
    // The path is y-down; the box's top is the smallest y.
    let top = (b.top() + dy).floor() as i32;
    let bottom = (b.bottom() + dy).ceil() as i32;
    (x0, -top, x1 - x0, bottom - top)
}
