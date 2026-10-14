#!/usr/bin/env bash
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
#
# Makes third_party/glow and third_party/wr_glyph_rasterizer (named in [patch.crates-io] in Cargo.toml)
# from the crates.io sources plus the WASI changes (a patch for one, a sed for the other). Run it once in a checkout
# before building for target_os = "wasi"; it does nothing if the folders exist.
set -eu
HERE="$(cd "$(dirname "$0")" && pwd)"
fetch() { # NAME VERSION
  [ -d "$HERE/$1" ] && return 0
  tmp="$(mktemp -d)"
  curl -sSfL "https://static.crates.io/crates/$1/$1-$2.crate" | tar xz -C "$tmp"
  mv "$tmp/$1-$2" "$HERE/$1"; rm -f "$HERE/$1/.cargo_vcs_info.json" "$HERE/$1/Cargo.toml.orig"
  rmdir "$tmp"
}
fetch glow 0.17.0
fetch wr_glyph_rasterizer 0.70.0
# glow: its lib.rs has CRLF line ends, so a patch file is brittle; use the OpenGL backend (not WebGL) on WASI.
sed -i -e 's/target_os = "emscripten"))\]/target_os = "emscripten", target_os = "wasi"))]/' \
       -e 's/not(target_os = "emscripten")))\]/not(target_os = "emscripten"), not(target_os = "wasi")))]/' "$HERE/glow/src/lib.rs"
(cd "$HERE/wr_glyph_rasterizer" && patch -p1 -s < "$HERE/wr_glyph_rasterizer-0.70.0-wasi.patch")
