/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! The connector Servo's HTTP client uses on WASI: connections through the `rb_net` imports instead of `tokio::net`.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use http::uri::{Scheme, Uri};
use hyper_util::client::legacy::connect::{Connected, Connection};
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tower::Service;

/// A connected `rb_net` socket.
pub struct WasiStream(rb_net::Stream);

impl AsyncRead for WasiStream {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for WasiStream {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

impl Connection for WasiStream {
    fn connected(&self) -> Connected {
        Connected::new()
    }
}

#[derive(Clone)]
pub struct WasiConnector {
    timeout: Duration,
}

impl WasiConnector {
    pub fn new(timeout: Duration) -> Self {
        WasiConnector { timeout }
    }
}

impl Service<Uri> for WasiConnector {
    type Response = TokioIo<WasiStream>;
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, dest: Uri) -> Self::Future {
        let timeout = self.timeout;
        Box::pin(async move {
            let host = dest
                .host()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no host in the address"))?
                .to_owned();
            let port = dest.port_u16().unwrap_or(if dest.scheme() == Some(&Scheme::HTTP) { 80 } else { 443 });
            // Resolving and connecting sleep in the host, so they run on a thread that may.
            let stream = tokio::task::spawn_blocking(move || rb_net::connect(&host, port, timeout))
                .await
                .map_err(io::Error::other)??;
            Ok(TokioIo::new(WasiStream(stream)))
        })
    }
}
