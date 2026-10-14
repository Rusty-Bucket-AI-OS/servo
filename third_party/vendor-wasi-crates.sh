#!/usr/bin/env bash
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
#
# Makes third_party/glow, third_party/wr_glyph_rasterizer, third_party/hyper-util, third_party/async-tungstenite and third_party/imsz (named in [patch.crates-io] in Cargo.toml)
# from the crates.io sources plus the WASI changes (patches, and a sed for glow). Run it once in a checkout before building
# for target_os = "wasi"; a folder that exists is left alone (delete it to make it again).
set -eu
HERE="$(cd "$(dirname "$0")" && pwd)"
fetch() { # NAME VERSION: returns 1 (and does nothing) if the folder is there
  [ -d "$HERE/$1" ] && return 1
  tmp="$(mktemp -d)"
  curl -sSfL "https://static.crates.io/crates/$1/$1-$2.crate" | tar xz -C "$tmp"
  mv "$tmp/$1-$2" "$HERE/$1"; rm -f "$HERE/$1/.cargo_vcs_info.json" "$HERE/$1/Cargo.toml.orig"
  rmdir "$tmp"
}
if fetch glow 0.17.0; then
  # glow: its lib.rs has CRLF line ends, so a patch file is brittle; use the OpenGL backend (not WebGL) on WASI.
  sed -i -e 's/target_os = "emscripten"))\]/target_os = "emscripten", target_os = "wasi"))]/' \
         -e 's/not(target_os = "emscripten")))\]/not(target_os = "emscripten"), not(target_os = "wasi")))]/' "$HERE/glow/src/lib.rs"
fi
if fetch wr_glyph_rasterizer 0.70.0; then
  # A platform module on skrifa and tiny-skia (outlines and coverage, no C): the one WASI uses.
  (cd "$HERE/wr_glyph_rasterizer" && patch -p1 -s < "$HERE/wr_glyph_rasterizer-0.70.0-skrifa.patch")
  mkdir -p "$HERE/wr_glyph_rasterizer/src/platform/skrifa"
  cp "$HERE/wr_glyph_rasterizer-skrifa-font.rs" "$HERE/wr_glyph_rasterizer/src/platform/skrifa/font.rs"
fi
if fetch hyper-util 0.1.21; then
  # Its HttpConnector is built on socket2, which has no WASI backend: leave the connector (and socket2) out on WASI,
  # where servo-net connects through the `rb_net` imports instead.
  (cd "$HERE/hyper-util" && patch -p1 -s < "$HERE/hyper-util-0.1.21-wasi.patch")
fi
if fetch async-tungstenite 0.35.0; then
  # tokio has no TcpStream::connect on WASI: the two connect_async helpers answer "unsupported" there.
  (cd "$HERE/async-tungstenite" && patch -p1 -s < "$HERE/async-tungstenite-0.35.0-wasi.patch")
fi
if fetch imsz 0.4.1; then
  # `impl Imsz for Stdin` calls the impl for `&Stdin`, which is there only on unix and windows.
  (cd "$HERE/imsz" && patch -p1 -s < "$HERE/imsz-0.4.1-wasi.patch")
fi
