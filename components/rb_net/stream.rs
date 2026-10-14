/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! A connected socket as a tokio stream. Two threads per connection do the blocking `fd_read` and `fd_write`;
//! the stream moves bytes through two bounded buffers and wakes the task that is waiting.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Read, Write};
use std::net::IpAddr;
use std::os::fd::FromRawFd;
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::sys;

/// How much one direction may hold before its producer waits (a reader that is slow stops the reader thread).
const BUFFER: usize = 256 * 1024;
const CHUNK: usize = 16 * 1024;

#[derive(Default)]
struct Half {
    data: VecDeque<u8>,
    /// The end of the data: the reader saw end of file or an error, or the writer was shut down.
    done: Option<io::ErrorKind>,
    finished: bool,
    waker: Option<Waker>,
}

struct Shared {
    incoming: Mutex<Half>,
    incoming_space: Condvar,
    outgoing: Mutex<Half>,
    outgoing_data: Condvar,
}

pub struct Stream {
    fd: u32,
    shared: Arc<Shared>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Stream {
    fn new(fd: u32) -> Stream {
        let shared = Arc::new(Shared {
            incoming: Mutex::default(),
            incoming_space: Condvar::new(),
            outgoing: Mutex::default(),
            outgoing_data: Condvar::new(),
        });
        // SAFETY: `fd` is a socket descriptor the host just gave this process and nothing else owns; the `File` closes it.
        let file = Arc::new(unsafe { File::from_raw_fd(fd as i32) });
        let (reader_file, writer_file) = (file.clone(), file);
        let (s1, s2) = (shared.clone(), shared.clone());
        thread::Builder::new().name("rb_net reader".into()).spawn(move || read_loop(&reader_file, &s1)).ok();
        thread::Builder::new().name("rb_net writer".into()).spawn(move || write_loop(&writer_file, &s2)).ok();
        Stream { fd, shared }
    }

    pub fn raw_fd(&self) -> u32 {
        self.fd
    }
}

fn read_loop(file: &File, shared: &Shared) {
    let mut buf = vec![0u8; CHUNK];
    loop {
        {
            let mut half = lock(&shared.incoming);
            while half.data.len() >= BUFFER && !half.finished {
                half = shared.incoming_space.wait(half).unwrap_or_else(|p| p.into_inner());
            }
            if half.finished {
                return;
            }
        }
        let result = (&*file).read(&mut buf);
        let mut half = lock(&shared.incoming);
        match result {
            Ok(0) => half.done = Some(io::ErrorKind::UnexpectedEof),
            Ok(n) => half.data.extend(&buf[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => half.done = Some(e.kind()),
        }
        let stop = half.done.is_some();
        if let Some(w) = half.waker.take() {
            w.wake();
        }
        if stop {
            return;
        }
    }
}

fn write_loop(file: &File, shared: &Shared) {
    loop {
        let chunk: Vec<u8> = {
            let mut half = lock(&shared.outgoing);
            while half.data.is_empty() && !half.finished {
                half = shared.outgoing_data.wait(half).unwrap_or_else(|p| p.into_inner());
            }
            if half.data.is_empty() {
                return; // shut down and drained
            }
            let n = half.data.len().min(CHUNK);
            half.data.drain(..n).collect()
        };
        let result = (&*file).write_all(&chunk);
        let mut half = lock(&shared.outgoing);
        if let Err(e) = result {
            half.done = Some(e.kind());
            half.data.clear();
            half.finished = true;
        }
        if let Some(w) = half.waker.take() {
            w.wake(); // room again (or the error to report)
        }
        if half.done.is_some() {
            return;
        }
    }
}

impl AsyncRead for Stream {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let mut half = lock(&self.shared.incoming);
        if !half.data.is_empty() {
            let n = half.data.len().min(buf.remaining());
            let (a, b) = half.data.as_slices();
            let first = a.len().min(n);
            buf.put_slice(&a[..first]);
            buf.put_slice(&b[..n - first]);
            half.data.drain(..n);
            self.shared.incoming_space.notify_one();
            return Poll::Ready(Ok(()));
        }
        match half.done {
            Some(io::ErrorKind::UnexpectedEof) => Poll::Ready(Ok(())), // end of file: nothing added to `buf`
            Some(kind) => Poll::Ready(Err(kind.into())),
            None => {
                half.waker = Some(cx.waker().clone());
                Poll::Pending
            },
        }
    }
}

impl AsyncWrite for Stream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        let mut half = lock(&self.shared.outgoing);
        if let Some(kind) = half.done {
            return Poll::Ready(Err(kind.into()));
        }
        if half.finished {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        let room = BUFFER.saturating_sub(half.data.len());
        if room == 0 {
            half.waker = Some(cx.waker().clone());
            return Poll::Pending;
        }
        let n = room.min(buf.len());
        half.data.extend(&buf[..n]);
        self.shared.outgoing_data.notify_one();
        Poll::Ready(Ok(n))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(())) // the writer thread sends as fast as the host takes it
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let mut half = lock(&self.shared.outgoing);
        half.finished = true;
        self.shared.outgoing_data.notify_one();
        Poll::Ready(Ok(()))
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        {
            let mut out = lock(&self.shared.outgoing);
            out.finished = true;
            out.data.clear();
        }
        {
            let mut inc = lock(&self.shared.incoming);
            inc.finished = true;
        }
        self.shared.outgoing_data.notify_one();
        self.shared.incoming_space.notify_one();
        // Ends a read that is asleep in the host and, with the writer's file, closes the connection.
        sys::shutdown(self.fd, 3);
    }
}

/// Resolves `host` (an address literal is used as it is) and connects to the first address that answers.
/// Blocking: call it from a thread that may sleep.
pub fn connect(host: &str, port: u16, timeout: Duration) -> io::Result<Stream> {
    let timeout_ms = timeout.as_millis().clamp(1, 120_000) as u32;
    let (name, literal) = match host.trim_start_matches('[').trim_end_matches(']').parse::<IpAddr>() {
        Ok(ip) => (String::new(), vec![ip]),
        Err(_) => (host.to_ascii_lowercase(), sys::resolve(&host.to_ascii_lowercase())?),
    };
    let mut last = io::Error::new(io::ErrorKind::NotFound, "no address");
    for ip in literal {
        match sys::connect_raw(ip, port, &name, timeout_ms) {
            Ok(fd) => return Ok(Stream::new(fd)),
            Err(e) => last = e,
        }
    }
    Err(last)
}
