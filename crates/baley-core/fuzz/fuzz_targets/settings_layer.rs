//! A settings file: the owner's `config.toml` or a repository's committed `baley.toml`.
#![no_main]

use std::path::PathBuf;

use baley_core::policy::{FileLayer, Schema, SettingsFile, parse_layer, read_project};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let file = SettingsFile {
        path: PathBuf::from("/fuzz/baley.toml"),
        bytes: data.to_vec(),
        digest: String::new(),
    };
    let _ = parse_layer(&file, FileLayer::Global, Schema::standard());
    let _ = parse_layer(&file, FileLayer::Project, Schema::standard());
    let _ = read_project(&file);
});
