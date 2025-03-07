/*
 * Converting MSHV Rust support to C bindings
 *
 * Copyright Microsoft, Corp. 2017
 *
 * Authors: ziqiaozhou@microsoft.com
 *
 * This is a PoC code that need to be rewritten in C dropping dependencies.
 *
 * This work is licensed under the terms of the GNU GPL, version 2 or later.
 * See the COPYING file in the top-level directory.
 *
 */

use cargo_metadata::MetadataCommand;
use std::env;
use std::path::Path;

fn main() {
    // Get the directory where the output files are to be placed
    let out_dir = env::var("OUT_DIR").unwrap();

    // Generate the header file using cbindgen
    let header_path = Path::new(&out_dir).join("../../../qemu-mshv.h");

    let mshv_bind_crate = "mshv-bindings";
    let metadata = MetadataCommand::new()
        .exec()
        .expect("Failed to run `cargo metadata`");
    let mshv_bind_dir = if let Some(package) = metadata
        .packages
        .iter()
        .find(|pkg| pkg.name == mshv_bind_crate)
    {
        package.manifest_path.parent().unwrap().as_std_path()
    } else {
        panic!("Crate '{}' not found", mshv_bind_crate);
    };

    let current_crate = env::var("CARGO_MANIFEST_DIR").unwrap();
    let config = cbindgen::Config::from_root_or_default(current_crate.clone());
    let f = mshv_bind_dir.join("src/x86_64/regs.rs");
    cbindgen::Builder::new()
        .with_config(config)
        .with_crate(current_crate)
        .with_src(f)
        .generate()
        .expect("unsable to generate bindings")
        .write_to_file(header_path);
}
