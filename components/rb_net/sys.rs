/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! The raw imports and their wire formats, wrapped. All the `unsafe` of the crate is here.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[link(wasm_import_module = "rb_net")]
unsafe extern "C" {
    #[link_name = "resolve"]
    fn rb_resolve(name: *const u8, len: u32, out: *mut u8, cap: u32, count: *mut u32) -> u32;
    #[link_name = "connect"]
    fn rb_connect(request: *const u8, len: u32, timeout_ms: u32, fd: *mut u32) -> u32;
    #[link_name = "shutdown"]
    fn rb_shutdown(fd: u32, how: u32) -> u32;
}

const ENTRY_LEN: usize = 20;
const MAX_RESOLVED: usize = 16;
const MAX_NAME: usize = 253;

/// WASI errno to the `io::Error` a network caller expects.
fn error(errno: u32) -> io::Error {
    use io::ErrorKind as K;
    let kind = match errno {
        2 => K::PermissionDenied,
        6 => K::WouldBlock,
        13 => K::ConnectionAborted,
        14 => K::ConnectionRefused,
        15 => K::ConnectionReset,
        23 => K::HostUnreachable,
        28 => K::InvalidInput,
        40 => K::NetworkUnreachable,
        44 => K::NotFound,
        64 => K::BrokenPipe,
        73 => K::TimedOut,
        _ => K::Other,
    };
    io::Error::new(kind, format!("rb_net: errno {errno}"))
}

fn decode(entry: &[u8]) -> Option<IpAddr> {
    let family = u32::from_le_bytes(entry[..4].try_into().ok()?);
    let b = &entry[4..ENTRY_LEN];
    match family {
        4 => Some(IpAddr::V4(Ipv4Addr::new(b[0], b[1], b[2], b[3]))),
        6 => Some(IpAddr::V6(Ipv6Addr::from(<[u8; 16]>::try_from(b).ok()?))),
        _ => None,
    }
}

/// The addresses of `name` (an address literal answers itself), at most 16.
pub fn resolve(name: &str) -> io::Result<Vec<IpAddr>> {
    if name.len() > MAX_NAME {
        return Err(error(28));
    }
    let mut out = [0u8; ENTRY_LEN * MAX_RESOLVED];
    let mut count = 0u32;
    // SAFETY: both buffers are live for the call and as long as the lengths given; the host checks them against the memory.
    let errno = unsafe { rb_resolve(name.as_ptr(), name.len() as u32, out.as_mut_ptr(), MAX_RESOLVED as u32, &mut count) };
    if errno != 0 {
        return Err(error(errno));
    }
    Ok(out.chunks_exact(ENTRY_LEN).take(count as usize).filter_map(decode).collect())
}

/// Opens a connection to `addr:port`. `host` is the name the address came from (empty for a literal).
pub fn connect_raw(addr: IpAddr, port: u16, host: &str, timeout_ms: u32) -> io::Result<u32> {
    let mut req = Vec::with_capacity(22 + host.len());
    req.push(1);
    req.push(if addr.is_ipv4() { 4 } else { 6 });
    req.extend_from_slice(&port.to_le_bytes());
    match addr {
        IpAddr::V4(a) => {
            req.extend_from_slice(&a.octets());
            req.extend_from_slice(&[0; 12]);
        },
        IpAddr::V6(a) => req.extend_from_slice(&a.octets()),
    }
    req.extend_from_slice(&(host.len() as u16).to_le_bytes());
    req.extend_from_slice(host.as_bytes());
    let mut fd = 0u32;
    // SAFETY: the request and the descriptor slot are live for the call; the host checks them against the memory.
    let errno = unsafe { rb_connect(req.as_ptr(), req.len() as u32, timeout_ms, &mut fd) };
    if errno != 0 { Err(error(errno)) } else { Ok(fd) }
}

/// 1 stop reading, 2 stop writing, 3 both.
pub fn shutdown(fd: u32, how: u32) {
    // SAFETY: plain integers; a descriptor that is not a socket answers an error, which is ignored.
    let _ = unsafe { rb_shutdown(fd, how) };
}
