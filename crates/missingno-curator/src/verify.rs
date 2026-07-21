//! Verification: fetch ROMs from sources, hash them, match against artifacts
//! and the user's local collection.

use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
};

use sha1::{Digest, Sha1};

pub fn sha1_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let response = ureq::get(url)
        .call()
        .map_err(|e| format!("download failed: {e}"))?;
    response
        .into_body()
        .read_to_vec()
        .map_err(|e| format!("read failed: {e}"))
}

const ROM_EXTENSIONS: [&str; 5] = ["gb", "gbc", "a26", "bin", "rom"];

/// sha1 → path index over the user's ROM directory.
#[derive(Default, Debug)]
pub struct RomIndex {
    pub by_sha1: HashMap<String, PathBuf>,
    pub scanned: usize,
}

impl RomIndex {
    pub fn scan(dir: &Path) -> io::Result<Self> {
        let mut index = Self::default();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let extension = path.extension().map(|e| e.to_string_lossy().to_lowercase());
                if !extension.is_some_and(|e| ROM_EXTENSIONS.contains(&e.as_str())) {
                    continue;
                }
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                index.by_sha1.insert(sha1_hex(&bytes), path);
                index.scanned += 1;
            }
        }
        Ok(index)
    }
}

/// Synchronous smoke-test: recognize → construct → produce frames → screenshot.
pub struct BootShot {
    pub frames_run: usize,
    pub frames_seen: usize,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn boot_check(
    filename_hint: &str,
    rom: &[u8],
    tv_standard: Option<String>,
    frames: usize,
) -> Result<BootShot, String> {
    let options = missingno_session::factory::LoadOptions {
        tv_standard,
        boot_rom: None,
    };
    let path = Path::new(filename_hint);
    let mut console = missingno_session::factory::create_console_with(path, rom, &options)
        .map_err(|e| format!("core rejected ROM: {e}"))?
        .ok_or("no core recognizes this ROM")?;
    let mut frames_seen = 0;
    for _ in 0..frames {
        if console.step_frame().display.is_some() {
            frames_seen += 1;
        }
    }
    let frame = console.screen_display().resolve_rgba();
    Ok(BootShot {
        frames_run: frames,
        frames_seen,
        width: frame.width,
        height: frame.height,
        rgba: frame.pixels.to_vec(),
    })
}
