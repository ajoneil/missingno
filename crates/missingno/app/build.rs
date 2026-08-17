use std::{fs, path::PathBuf};

/// Catalogue trees, one per console; the archive path's leading segment records
/// which console a manifest belongs to.
const CONSOLES: [&str; 4] = ["gb", "gbc", "sg1000", "vcs"];

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let gamedb_dir = manifest_dir.join("../../../missingno-gamedb/data");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let archive_path = out_dir.join("gamedb.tar.zst");

    // Rerun if any console tree changes.
    for console in CONSOLES {
        println!(
            "cargo:rerun-if-changed={}",
            gamedb_dir.join(console).display()
        );
    }

    if !gamedb_dir.join(CONSOLES[0]).is_dir() {
        panic!(
            "game database not found at {} — run: git submodule update --init",
            gamedb_dir.display()
        );
    }

    // Build a tar of every console tree, keying entries as {console}/{slug}/{file}.
    let tar_data = Vec::new();
    let mut builder = tar::Builder::new(tar_data);

    let mut file_count = 0;
    for console in CONSOLES {
        let Ok(entries) = fs::read_dir(gamedb_dir.join(console)) else {
            continue;
        };
        let mut dirs: Vec<_> = entries.flatten().filter(|e| e.path().is_dir()).collect();
        dirs.sort_by_key(|e| e.file_name());

        for dir in dirs {
            let slug_str = dir.file_name().to_string_lossy().into_owned();
            if let Ok(files) = fs::read_dir(dir.path()) {
                for file in files.flatten() {
                    let path = file.path();
                    if path.extension().map(|e| e == "ron").unwrap_or(false) {
                        let archive_name = format!(
                            "{console}/{slug_str}/{}",
                            file.file_name().to_string_lossy()
                        );
                        builder.append_path_with_name(&path, &archive_name).unwrap();
                        file_count += 1;
                    }
                }
            }
        }
    }

    let tar_data = builder.into_inner().unwrap();
    let compressed = zstd::encode_all(tar_data.as_slice(), 19).unwrap();
    fs::write(&archive_path, &compressed).unwrap();

    eprintln!(
        "cargo:warning=GameDB: {} files, {} bytes tar, {} bytes compressed",
        file_count,
        tar_data.len(),
        compressed.len(),
    );
}
