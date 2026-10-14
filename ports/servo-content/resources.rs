/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! The page resources Servo reads at run time (error pages, the public suffix list, the HSTS preload list, ...), baked into the
//! binary from the files of `servo-default-resources`. This crate registers its own reader instead of depending on that crate's:
//! on WASI the linker drops the registration of a crate nothing refers to by name.

use std::path::PathBuf;

use embedder_traits::resources::{Resource, ResourceReaderMethods};

struct BakedIn;

impl ResourceReaderMethods for BakedIn {
    fn sandbox_access_files(&self) -> Vec<PathBuf> {
        vec![]
    }

    fn sandbox_access_files_dirs(&self) -> Vec<PathBuf> {
        vec![]
    }

    fn read(&self, file: Resource) -> Vec<u8> {
        macro_rules! resource {
            ($name:literal) => {
                &include_bytes!(concat!("../../components/default-resources/resources/", $name))[..]
            };
        }
        match file {
            Resource::BluetoothBlocklist => resource!("gatt_blocklist.txt"),
            Resource::DomainList => resource!("public_domains.txt"),
            Resource::HstsPreloadList => resource!("hsts_preload.fstmap"),
            Resource::BadCertHTML => resource!("badcert.html"),
            Resource::NetErrorHTML => resource!("neterror.html"),
            Resource::BrokenImageIcon => resource!("rippy.png"),
            Resource::CrashHTML => resource!("crash.html"),
            Resource::DirectoryListingHTML => resource!("directory-listing.html"),
            Resource::AboutMemoryHTML => resource!("about-memory.html"),
            Resource::DebuggerJS => resource!("debugger.js"),
            Resource::JsonViewerHTML => resource!("json-viewer.html"),
        }
        .to_owned()
    }
}

embedder_traits::submit_resource_reader!(&BakedIn);
