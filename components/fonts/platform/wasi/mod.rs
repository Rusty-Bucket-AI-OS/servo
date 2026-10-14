//! Spike stub: a font platform for wasm32-wasip1 with no behavior, only the interface.
pub mod font {
    use app_units::Au;
    use euclid::default::Rect;
    use fonts_traits::{FontIdentifier, FontTemplateDescriptor, LocalFontIdentifier};
    use read_fonts::types::Tag;
    use webrender_api::{FontInstanceFlags, FontVariation};

    use crate::FontData;
    use crate::font::{FontMetrics, FontTableMethods, FractionalPixel, PlatformFontMethods};
    use crate::glyph::GlyphId;

    #[derive(Debug)]
    pub struct FontTable;
    impl FontTableMethods for FontTable {
        fn buffer(&self) -> &[u8] {
            &[]
        }
    }
    #[derive(Debug)]
    pub struct PlatformFont;
    impl PlatformFontMethods for PlatformFont {
        fn new_from_local_font_identifier(_: LocalFontIdentifier, _: Option<Au>, _: bool) -> Result<PlatformFont, &'static str> {
            Err("stub")
        }
        fn new_from_data(_: FontIdentifier, _: &FontData, _: Option<Au>, _: bool) -> Result<PlatformFont, &'static str> {
            Err("stub")
        }
        fn copy_with_variations(self, _: &FontIdentifier, _: &[FontVariation]) -> Result<Self, &'static str> {
            Err("stub")
        }
        fn descriptor(&self) -> FontTemplateDescriptor {
            unimplemented!()
        }
        fn glyph_index(&self, _: char) -> Option<GlyphId> {
            None
        }
        fn glyph_h_advance(&self, _: GlyphId) -> Option<FractionalPixel> {
            None
        }
        fn glyph_h_kerning(&self, _: GlyphId, _: GlyphId) -> FractionalPixel {
            0.0
        }
        fn metrics(&self) -> FontMetrics {
            unimplemented!()
        }
        fn table_for_tag(&self, _: Tag) -> Option<FontTable> {
            None
        }
        fn typographic_bounds(&self, _: GlyphId) -> Rect<f32> {
            Rect::zero()
        }
        fn webrender_font_instance_flags(&self) -> FontInstanceFlags {
            FontInstanceFlags::empty()
        }
        fn variations(&self) -> &[FontVariation] {
            &[]
        }
    }
}
pub mod font_list {
    use style::values::computed::font::GenericFontFamily;
    use crate::{FallbackFontSelectionOptions, FontTemplate, LowercaseFontFamilyName};

    pub(crate) fn for_each_available_family<F: FnMut(&str)>(_: F) {}
    pub(crate) fn for_each_variation<F: FnMut(FontTemplate)>(_: &str, _: F) {}
    pub fn fallback_font_families(_: FallbackFontSelectionOptions) -> Vec<&'static str> {
        vec![]
    }
    pub(crate) fn default_system_generic_font_family(_: GenericFontFamily) -> LowercaseFontFamilyName {
        LowercaseFontFamilyName::from("sans-serif")
    }
}
