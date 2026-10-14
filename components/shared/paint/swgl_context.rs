/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! A [`RenderingContext`] for WASI: WebRender's own software OpenGL (`swgl`) drawing into a buffer in memory.
//! There is no window system and no GPU: the frame ends up in [`SoftwareRenderingContext::with_frame`], BGRA with the
//! bottom row first (OpenGL's order), which the embedder (the browser shell) copies out.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use dpi::PhysicalSize;
use gleam::gl::{self, Gl};
use image::RgbaImage;
use webrender_api::units::DeviceIntRect;

use crate::rendering_context::{Error, RenderingContext};

pub struct SoftwareRenderingContext {
    gl: Rc<swgl::Context>,
    size: Cell<PhysicalSize<u32>>,
    /// The default framebuffer of `gl`: `size.width * size.height * 4` bytes of BGRA, rows bottom to top.
    frame: RefCell<Vec<u8>>,
}

impl SoftwareRenderingContext {
    pub fn new(size: PhysicalSize<u32>) -> Result<Self, Error> {
        let context = Self {
            gl: Rc::new(swgl::Context::create()),
            size: Cell::new(size),
            frame: RefCell::new(Vec::new()),
        };
        context.gl.make_current();
        context.attach_frame(size);
        Ok(context)
    }

    /// Allocates the pixel buffer for `size` and makes it the context's default framebuffer.
    fn attach_frame(&self, size: PhysicalSize<u32>) {
        let mut frame = self.frame.borrow_mut();
        *frame = vec![0u8; size.width as usize * size.height as usize * 4];
        // `frame` is not touched again until the next call to this function, which gives the context a new default
        // framebuffer at once, and it is as large as the stride times the height given here.
        self.gl.init_default_framebuffer(
            0,
            0,
            size.width as i32,
            size.height as i32,
            (size.width * 4) as i32,
            frame.as_mut_ptr() as *mut _,
        );
    }

    /// Calls `f` with the current frame: BGRA, `width * 4` bytes a row, the bottom row first.
    pub fn with_frame<R>(&self, f: impl FnOnce(&[u8], PhysicalSize<u32>) -> R) -> R {
        self.gl.make_current();
        f(&self.frame.borrow(), self.size.get())
    }
}

impl RenderingContext for SoftwareRenderingContext {
    fn prepare_for_rendering(&self) {
        self.gl.make_current();
        self.gl.bind_framebuffer(gl::FRAMEBUFFER, 0);
    }

    fn read_to_image(&self, source: DeviceIntRect) -> Option<RgbaImage> {
        let size = self.size.get();
        let (w, h) = (size.width as i32, size.height as i32);
        let rect = source.intersection(&DeviceIntRect::from_size(euclid::Size2D::new(w, h)))?;
        if rect.is_empty() {
            return None;
        }
        let frame = self.frame.borrow();
        let mut out = RgbaImage::new(rect.width() as u32, rect.height() as u32);
        for (j, row) in out.rows_mut().enumerate() {
            // `source` is in window coordinates (top row first); the buffer has the bottom row first.
            let src_y = (h - 1 - (rect.min.y + j as i32)) as usize;
            for (i, px) in row.enumerate() {
                let at = (src_y * w as usize + (rect.min.x as usize + i)) * 4;
                px.0 = [frame[at + 2], frame[at + 1], frame[at], frame[at + 3]];
            }
        }
        Some(out)
    }

    fn size(&self) -> PhysicalSize<u32> {
        self.size.get()
    }

    fn resize(&self, size: PhysicalSize<u32>) {
        if size == self.size.get() || size.width == 0 || size.height == 0 {
            return;
        }
        self.gl.make_current();
        self.size.set(size);
        self.attach_frame(size);
    }

    fn present(&self) {}

    fn make_current(&self) -> Result<(), Error> {
        self.gl.make_current();
        Ok(())
    }

    fn gleam_gl_api(&self) -> Rc<dyn Gl> {
        self.gl.clone()
    }
}
