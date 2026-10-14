/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

#![deny(unsafe_code)]

use std::rc::Rc;
#[cfg(not(target_os = "wasi"))]
use std::sync::Arc;

use dpi::PhysicalSize;
use embedder_traits::RefreshDriver;
use euclid::Size2D;
use euclid::default::Size2D as UntypedSize2D;
use gleam::gl::Gl;
use image::RgbaImage;
#[cfg(not(target_os = "wasi"))]
pub use surfman::Error;
#[cfg(not(target_os = "wasi"))]
use surfman::{Connection, Surface, SurfaceTexture};
use webrender_api::units::{DeviceIntRect, DevicePixel};

// WASI has no OpenGL context manager (`surfman`); the trait keeps the same shape with opaque stand-ins
// and the contexts that need `surfman` are not built. See docs/developer/servo-spike-patches.md.
#[cfg(target_os = "wasi")]
pub use crate::surfman_shim::{Connection, Error, Surface, SurfaceTexture};

/// The `RenderingContext` trait defines a set of methods for managing
/// an OpenGL or GLES rendering context.
/// Implementors of this trait are responsible for handling the creation,
/// management, and destruction of the rendering context and its associated
/// resources.
pub trait RenderingContext {
    /// Prepare this [`RenderingContext`] to be rendered upon by Servo. For instance,
    /// by binding a framebuffer to the current OpenGL context.
    fn prepare_for_rendering(&self) {}
    /// Read the contents of this [`Renderingcontext`] into an in-memory image. If the
    /// image cannot be read (for instance, if no rendering has taken place yet), then
    /// `None` is returned.
    ///
    /// In a double-buffered [`RenderingContext`] this is expected to read from the back
    /// buffer. That means that once Servo renders to the context, this should return those
    /// results, even before [`RenderingContext::present`] is called.
    fn read_to_image(&self, source_rectangle: DeviceIntRect) -> Option<RgbaImage>;
    /// Get the current size of this [`RenderingContext`].
    fn size(&self) -> PhysicalSize<u32>;
    /// Get the current size of this [`RenderingContext`] as [`Size2D`].
    fn size2d(&self) -> Size2D<u32, DevicePixel> {
        let size = self.size();
        Size2D::new(size.width, size.height)
    }
    /// Resizes the rendering surface to the given size.
    fn resize(&self, size: PhysicalSize<u32>);
    /// Presents the rendered frame to the screen. In a double-buffered context, this would
    /// swap buffers.
    fn present(&self);
    /// Makes the context the current OpenGL context for this thread.
    /// After calling this function, it is valid to use OpenGL rendering
    /// commands.
    fn make_current(&self) -> Result<(), Error>;
    /// Returns the `gleam` version of the OpenGL or GLES API.
    fn gleam_gl_api(&self) -> Rc<dyn gleam::gl::Gl>;
    /// Returns the OpenGL or GLES API (for WebGL, which WASI does not have).
    #[cfg(not(target_os = "wasi"))]
    fn glow_gl_api(&self) -> Arc<glow::Context>;
    /// Creates a texture from a given surface and returns the surface texture,
    /// the OpenGL texture object, and the size of the surface. Default to `None`.
    fn create_texture(
        &self,
        _surface: Surface,
    ) -> Option<(SurfaceTexture, u32, UntypedSize2D<i32>)> {
        None
    }
    /// Destroys the texture and returns the surface. Default to `None`.
    fn destroy_texture(&self, _surface_texture: SurfaceTexture) -> Option<Surface> {
        None
    }
    /// The connection to the display server for WebGL. Default to `None`.
    fn connection(&self) -> Option<Connection> {
        None
    }
    /// Return the [`RefreshDriver`] for this [`RenderingContext`]. If `None` is returned,
    /// then the default timer-based [`RefreshDriver`] will be used.
    fn refresh_driver(&self) -> Option<Rc<dyn RefreshDriver>> {
        None
    }
}


#[cfg(not(target_os = "wasi"))]
mod surfman_contexts;
#[cfg(not(target_os = "wasi"))]
pub use surfman_contexts::{OffscreenRenderingContext, SoftwareRenderingContext, WindowRenderingContext};
// On WASI the one rendering context is WebRender's software OpenGL drawing into memory.
#[cfg(target_os = "wasi")]
pub use crate::swgl_context::SoftwareRenderingContext;
