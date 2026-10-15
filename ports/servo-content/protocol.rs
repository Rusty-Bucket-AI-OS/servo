/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! `rbcp`: what the content process says on its standard output and hears on its standard input. The same bytes as
//! `spikes/servo/rb-host/src/rbcp.rs` in the Rusty Bucket repository, where the format is described: `[kind u8][length u32 LE][body]`.

use std::io::{self, Read, Write};

pub const VERSION: u16 = 2;

pub enum Command {
    Navigate(String),
    /// 0 back, 1 forward, 2 reload, 3 stop.
    Nav(u8),
    Resize(u16, u16),
    /// `down` is the wire's state byte: 0 up, 1 down, 2 down again (`repeat`, an auto-repeat). `code` is the physical key's
    /// UI Events name (`KeyboardEvent.code`): `KeyQ` for the key that types `a` on a French keyboard.
    Key { down: bool, repeat: bool, mods: u8, key: String, text: String, code: String },
    /// kind 0 move, 1 down, 2 up; button 0 primary, 1 middle, 2 secondary.
    Pointer { kind: u8, x: i16, y: i16, button: u8 },
    Wheel { x: i16, y: i16, dx: i16, dy: i16 },
    Quit,
}

fn bad() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "rbcp: bad message")
}

/// Reads one command; `Ok(None)` when the host closed the stream.
pub fn read_command(r: &mut impl Read) -> io::Result<Option<Command>> {
    let mut head = [0u8; 5];
    match r.read_exact(&mut head) {
        Ok(()) => {},
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if len > 1 << 20 {
        return Err(bad());
    }
    let mut b = vec![0u8; len];
    r.read_exact(&mut b)?;
    let i16at = |i: usize| b.get(i..i + 2).map(|s| i16::from_le_bytes([s[0], s[1]])).ok_or_else(bad);
    let text = |s: &[u8]| String::from_utf8_lossy(s).into_owned();
    Ok(Some(match head[0] {
        1 => Command::Navigate(text(&b)),
        2 => Command::Nav(*b.first().ok_or_else(bad)?),
        3 => Command::Resize(i16at(0)? as u16, i16at(2)? as u16),
        4 => {
            let state = *b.first().ok_or_else(bad)?;
            let (down, repeat) = (state != 0, state == 2);
            let (mods, klen) = (*b.get(1).ok_or_else(bad)?, usize::from(*b.get(2).ok_or_else(bad)?));
            let key = text(b.get(3..3 + klen).ok_or_else(bad)?);
            let tlen = usize::from(*b.get(3 + klen).ok_or_else(bad)?);
            let typed = text(b.get(4 + klen..4 + klen + tlen).ok_or_else(bad)?);
            let clen = usize::from(*b.get(4 + klen + tlen).ok_or_else(bad)?);
            let code = text(b.get(5 + klen + tlen..5 + klen + tlen + clen).ok_or_else(bad)?);
            Command::Key { down, repeat, mods, key, text: typed, code }
        },
        5 => Command::Pointer { kind: *b.first().ok_or_else(bad)?, x: i16at(1)?, y: i16at(3)?, button: *b.get(5).ok_or_else(bad)? },
        6 => Command::Wheel { x: i16at(0)?, y: i16at(2)?, dx: i16at(4)?, dy: i16at(6)? },
        7 => Command::Quit,
        _ => return Err(bad()),
    }))
}

fn send(out: &mut impl Write, kind: u8, parts: &[&[u8]]) -> io::Result<()> {
    let len: usize = parts.iter().map(|p| p.len()).sum();
    out.write_all(&[kind])?;
    out.write_all(&(len as u32).to_le_bytes())?;
    for p in parts {
        out.write_all(p)?;
    }
    out.flush()
}

pub fn hello(out: &mut impl Write, w: u16, h: u16) -> io::Result<()> {
    send(out, 1, &[&VERSION.to_le_bytes(), &w.to_le_bytes(), &h.to_le_bytes()])
}

/// A rectangle of the picture: `w * h` BGRA pixels, the top row first.
pub fn rect(out: &mut impl Write, x: u16, y: u16, w: u16, h: u16, bgra: &[u8]) -> io::Result<()> {
    send(out, 2, &[&x.to_le_bytes(), &y.to_le_bytes(), &w.to_le_bytes(), &h.to_le_bytes(), bgra])
}

pub fn title(out: &mut impl Write, t: &str) -> io::Result<()> {
    send(out, 3, &[t.as_bytes()])
}

pub fn url(out: &mut impl Write, u: &str) -> io::Result<()> {
    send(out, 4, &[u.as_bytes()])
}

pub fn status(out: &mut impl Write, loading: bool, can_back: bool, can_forward: bool) -> io::Result<()> {
    send(out, 5, &[&[u8::from(loading), u8::from(can_back), u8::from(can_forward)]])
}

pub fn error(out: &mut impl Write, t: &str) -> io::Result<()> {
    send(out, 6, &[t.as_bytes()])
}
