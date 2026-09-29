use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The crates whose sources make up the emulated system: this one and the
/// path dependency it builds on.
const SOURCES: [&str; 2] = [".", "../../missingno/core"];

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files_under(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let mut digest = Sha256::new();
    for krate in SOURCES {
        let root = manifest_dir.join(krate);
        let manifest = root.join("Cargo.toml");
        let src = root.join("src");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rerun-if-changed={}", src.display());

        let mut files = vec![manifest];
        files_under(&src, &mut files);
        files.sort();
        for file in files {
            // Keyed by the path within the crate, so a checkout's location doesn't count.
            let name = file
                .strip_prefix(&root)
                .unwrap()
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            let contents = fs::read(&file).unwrap();
            digest.update(krate.as_bytes());
            digest.update(name.len().to_le_bytes());
            digest.update(name.as_bytes());
            digest.update(contents.len().to_le_bytes());
            digest.update(&contents);
        }
    }
    let hex: String = digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    println!("cargo:rustc-env=MISSINGNO_GB_SOURCE_DIGEST={hex}");
}
