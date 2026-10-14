/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! The network on Rusty Bucket (WASI): the four `rb_net` imports (`resolve`, `connect`, `shutdown`, `peer_addr`) and
//! a `Stream` over a connected socket descriptor that tokio can use. Sockets are descriptors in the process's table, so
//! reading and writing are `fd_read` and `fd_write`, which sleep in the host; a reader thread and a writer thread
//! per connection turn that into `AsyncRead` and `AsyncWrite`. The wire formats and the checks the host makes are the
//! spec in `docs/developer/servo-spike-net.md` of the Rusty Bucket repository. On other targets the crate is empty.

#[cfg(target_os = "wasi")]
mod stream;
#[cfg(target_os = "wasi")]
mod sys;

#[cfg(target_os = "wasi")]
pub use stream::{Stream, connect};
#[cfg(target_os = "wasi")]
pub use sys::resolve;
