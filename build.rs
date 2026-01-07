//! Build script for generating C headers via cbindgen

use std::env;
use std::path::PathBuf;

fn main() {
    let crate_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let output_dir = PathBuf::from(&crate_dir).join("include");
    
    // Create include directory if it doesn't exist
    std::fs::create_dir_all(&output_dir).expect("Failed to create include directory");
    
    let output_file = output_dir.join("gooey.h");
    
    // Generate C header using cbindgen
    let config = cbindgen::Config::from_file("cbindgen.toml")
        .expect("Failed to read cbindgen.toml");
    
    cbindgen::Builder::new()
        .with_crate(crate_dir)
        .with_config(config)
        .generate()
        .expect("Failed to generate C bindings")
        .write_to_file(&output_file);
    
    println!("cargo:rerun-if-changed=src/ffi.rs");
    println!("cargo:rerun-if-changed=cbindgen.toml");
}
