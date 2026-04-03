//! Delta-compressed save archive using zstd dictionary mode.
//!
//! Format: append-only binary file with entries. Each entry is either a
//! standalone keyframe or a delta compressed against the previous entry.
//!
//! ```text
//! [magic: 4 bytes "MNSA"]
//! [version: u8]
//! [entry]*
//!
//! entry:
//!   [flags: u8]           — 0x00 = delta, 0x01 = keyframe
//!   [uncompressed_len: u32 LE]
//!   [compressed_len: u32 LE]
//!   [compressed_data: compressed_len bytes]
//! ```
//!
//! Keyframes are standalone zstd-compressed. Deltas use the previous
//! entry's decompressed data as a zstd dictionary.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

const MAGIC: &[u8; 4] = b"MNSA";
const VERSION: u8 = 1;
const KEYFRAME_INTERVAL: usize = 32;
const ZSTD_LEVEL: i32 = 3;

const FLAG_DELTA: u8 = 0x00;
const FLAG_KEYFRAME: u8 = 0x01;

/// Append a save to the archive. Creates the archive if it doesn't exist.
/// `entry_index` is the 0-based index of this entry (used to decide keyframe placement).
pub fn append_save(archive_path: &Path, data: &[u8], entry_index: usize, prev_data: Option<&[u8]>) -> io::Result<()> {
    let is_keyframe = entry_index % KEYFRAME_INTERVAL == 0 || prev_data.is_none();
    let exists = archive_path.exists();

    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(true)
        .open(archive_path)?;

    // Write header if new file
    if !exists {
        file.write_all(MAGIC)?;
        file.write_all(&[VERSION])?;
    }

    let (flag, compressed) = if is_keyframe {
        let mut compressor = zstd::bulk::Compressor::new(ZSTD_LEVEL)?;
        let compressed = compressor.compress(data)?;
        (FLAG_KEYFRAME, compressed)
    } else {
        let dict = prev_data.unwrap();
        let mut compressor = zstd::bulk::Compressor::with_dictionary(ZSTD_LEVEL, dict)?;
        let compressed = compressor.compress(data)?;
        (FLAG_DELTA, compressed)
    };

    // Write entry
    file.write_all(&[flag])?;
    file.write_all(&(data.len() as u32).to_le_bytes())?;
    let compressed_len: u32 = compressed.len() as u32;
    file.write_all(&compressed_len.to_le_bytes())?;
    file.write_all(&compressed)?;
    file.flush()?;

    // fsync for crash safety
    file.sync_all()?;

    Ok(())
}

/// Read a specific entry from the archive by index.
/// Decompresses the delta chain from the nearest keyframe.
pub fn read_save(archive_path: &Path, target_index: usize) -> io::Result<Vec<u8>> {
    let file_data = fs::read(archive_path)?;
    let entries = parse_entries(&file_data)?;

    if target_index >= entries.len() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("save index {target_index} not found in archive ({} entries)", entries.len()),
        ));
    }

    // Find the nearest keyframe at or before target
    let keyframe_idx = (0..=target_index)
        .rev()
        .find(|&i| entries[i].is_keyframe)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no keyframe found"))?;

    // Decompress keyframe
    let uncompressed_len = entries[keyframe_idx].uncompressed_len as usize;
    let mut decompressor = zstd::bulk::Decompressor::new()?;
    let mut current = decompressor.decompress(entries[keyframe_idx].compressed, uncompressed_len)?;

    // Apply deltas forward
    for i in (keyframe_idx + 1)..=target_index {
        let entry = &entries[i];
        let len = entry.uncompressed_len as usize;
        if entry.is_keyframe {
            decompressor = zstd::bulk::Decompressor::new()?;
            current = decompressor.decompress(entry.compressed, len)?;
        } else {
            decompressor = zstd::bulk::Decompressor::with_dictionary(&current)?;
            current = decompressor.decompress(entry.compressed, len)?;
        }
    }

    Ok(current)
}

/// Count the number of entries in the archive.
pub fn entry_count(archive_path: &Path) -> usize {
    let Ok(data) = fs::read(archive_path) else {
        return 0;
    };
    parse_entries(&data).map(|e| e.len()).unwrap_or(0)
}


struct ArchiveEntry<'a> {
    is_keyframe: bool,
    #[allow(dead_code)]
    uncompressed_len: u32,
    compressed: &'a [u8],
}

fn parse_entries(data: &[u8]) -> io::Result<Vec<ArchiveEntry<'_>>> {
    if data.len() < 5 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "archive too small"));
    }
    if &data[0..4] != MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "bad magic"));
    }
    if data[4] != VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported version {}", data[4]),
        ));
    }

    let mut pos = 5;
    let mut entries = Vec::new();

    while pos < data.len() {
        // Need at least 9 bytes for flag + uncompressed_len + compressed_len
        if pos + 9 > data.len() {
            break; // Truncated entry (crash recovery) — ignore
        }

        let flag = data[pos];
        let uncompressed_len = u32::from_le_bytes(data[pos + 1..pos + 5].try_into().unwrap());
        let compressed_len = u32::from_le_bytes(data[pos + 5..pos + 9].try_into().unwrap()) as usize;
        pos += 9;

        if pos + compressed_len > data.len() {
            break; // Truncated data (crash recovery) — ignore
        }

        entries.push(ArchiveEntry {
            is_keyframe: flag == FLAG_KEYFRAME,
            uncompressed_len,
            compressed: &data[pos..pos + compressed_len],
        });

        pos += compressed_len;
    }

    Ok(entries)
}

/// Compress all individual .sav files in the saves/ directory into the archive,
/// then remove the individual files. Used for migration.
pub fn migrate_individual_saves(
    archive_path: &Path,
    saves_dir: &Path,
    save_ids: &[String],
) -> io::Result<()> {
    let existing_count = entry_count(archive_path);
    let mut prev_data: Option<Vec<u8>> = None;

    // If archive already has entries, read the last one for delta base
    if existing_count > 0 {
        prev_data = read_save(archive_path, existing_count - 1).ok();
    }

    for (i, id) in save_ids.iter().enumerate() {
        let sav_path = saves_dir.join(format!("{id}.sav"));
        if !sav_path.exists() {
            continue;
        }

        let data = fs::read(&sav_path)?;
        let entry_index = existing_count + i;
        append_save(archive_path, &data, entry_index, prev_data.as_deref())?;

        prev_data = Some(data);

        // Remove the individual file after successful archive
        let _ = fs::remove_file(&sav_path);
    }

    Ok(())
}
