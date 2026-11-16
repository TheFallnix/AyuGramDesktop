use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let mut build = cxx_build::bridge("src/lib.rs");
    build
        .std("c++20")
        .flag_if_supported("-Wno-maybe-uninitialized")
        .compile("ayu_ffi_cxx");
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-env-changed=AYU_CXX_HEADER_OUT_DIR");

    if let Some(export_dir) = env::var_os("AYU_CXX_HEADER_OUT_DIR") {
        if let Some(out_dir) = env::var_os("OUT_DIR") {
            let src_inc = PathBuf::from(out_dir).join("cxxbridge").join("include");
            let dst_inc = PathBuf::from(export_dir);
            let _ = copy_header(&src_inc, &dst_inc, "rust/cxx.h");
            let _ = copy_header(&src_inc, &dst_inc, "ayu-ffi/src/lib.rs.h");
        }
    }
}

fn copy_header(src_root: &Path, dst_root: &Path, rel: &str) -> std::io::Result<()> {
    let dst = dst_root.join(rel);
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(src_root.join(rel), dst)?;
    Ok(())
}
