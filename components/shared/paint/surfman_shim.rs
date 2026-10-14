/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Stand-ins for the few `surfman` types that appear in `paint_api`'s interfaces, for targets
//! where `surfman` (an OpenGL context manager with no WASI or CPU backend) is not built.
//! Nothing here creates a context; a software `RenderingContext` for WASI replaces it.

#[derive(Clone, Debug)]
pub struct Connection;

#[derive(Clone, Debug)]
pub struct Adapter;

#[derive(Debug)]
pub struct Surface;

#[derive(Debug)]
pub struct SurfaceTexture;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Failed,
    UnsupportedOnThisPlatform,
}
