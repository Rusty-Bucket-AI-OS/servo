/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! The font platform for WASI (Rusty Bucket): fonts are files read through the process's folders, outlines and metrics come
//! from `skrifa`, and there is no system font database and no FreeType. Shaping is HarfBuzz as on every platform.
//! Fonts are looked for in the folders named by the `RB_FONT_DIRS` environment variable (separated by `:`; default `/fonts`).

pub mod font;
pub mod font_list;
