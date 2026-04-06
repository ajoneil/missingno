use std::{fs, path::PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let gamedb_dir = manifest_dir.join("../../missingno-gamedb/games");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let archive_path = out_dir.join("gamedb.tar.zst");

    // Rerun if the gamedb changes
    println!("cargo:rerun-if-changed={}", gamedb_dir.display());

    if !gamedb_dir.is_dir() {
        eprintln!("cargo:warning=Game database not found at {}", gamedb_dir.display());
        eprintln!("cargo:warning=Run: git submodule update --init");
        // Write an empty archive so the build doesn't fail
        let tar_data = Vec::new();
        let builder = tar::Builder::new(tar_data);
        let tar_data = builder.into_inner().unwrap();
        let compressed = zstd::encode_all(tar_data.as_slice(), 19).unwrap();
        fs::write(&archive_path, compressed).unwrap();
        return;
    }

    // Build a tar archive of the games directory
    let tar_data = Vec::new();
    let mut builder = tar::Builder::new(tar_data);

    let mut file_count = 0;
    if let Ok(entries) = fs::read_dir(&gamedb_dir) {
        let mut dirs: Vec<_> = entries.flatten().filter(|e| e.path().is_dir()).collect();
        dirs.sort_by_key(|e| e.file_name());

        for dir in dirs {
            let slug = dir.file_name();
            let slug_str = slug.to_string_lossy();

            if let Ok(files) = fs::read_dir(dir.path()) {
                for file in files.flatten() {
                    let path = file.path();
                    if path.extension().map(|e| e == "ron").unwrap_or(false) {
                        let archive_name = format!("{}/{}", slug_str, file.file_name().to_string_lossy());
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
