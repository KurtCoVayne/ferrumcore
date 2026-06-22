//! Compile the canonical `.proto` IDL into prost types at build time using the **pure-Rust** `protox` compiler — no
//! system `protoc` toolchain dependency (reproducible builds, ADR-0002). The generated module is `include!`d by lib.rs.

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("proto");
    let files = [
        root.join("ferrumcore/core_ledger/v1/money.proto"),
        root.join("ferrumcore/core_ledger/v1/loan.proto"),
    ];
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
    }

    // protox parses + type-checks the IDL and yields a FileDescriptorSet prost-build can consume directly.
    let descriptors = protox::compile(&files, [&root])?;

    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    prost_build::Config::new()
        .file_descriptor_set_path(out_dir.join("descriptors.bin"))
        .compile_fds(descriptors)?;
    Ok(())
}
