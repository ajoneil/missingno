//! ROM patch application: IPS and BPS, the formats Game Boy hacks ship in.
//!
//! IPS carries no checksums or base identity — it applies blind, by
//! convention. BPS embeds source/target sizes and CRC32s; a source mismatch
//! is an error rather than a corrupt boot.

use std::fmt;
use std::path::Path;

#[derive(Debug, PartialEq, Eq)]
pub enum PatchError {
    Malformed(&'static str),
    /// The base ROM isn't the one the patch was made for.
    SourceMismatch,
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PatchError::Malformed(what) => write!(f, "malformed patch: {what}"),
            PatchError::SourceMismatch => write!(f, "patch does not match this ROM"),
        }
    }
}

/// Apply a `.ips`/`.bps` sitting next to the ROM (RetroArch-style
/// softpatching). Returns the ROM unchanged when no patch file exists or the
/// patch fails to apply.
pub fn soft_patch(rom_path: &Path, rom: Vec<u8>) -> Vec<u8> {
    let candidates = [
        (rom_path.with_extension("bps"), apply_bps as ApplyFn),
        (rom_path.with_extension("ips"), apply_ips as ApplyFn),
    ];
    for (path, apply) in candidates {
        let Ok(patch) = std::fs::read(&path) else {
            continue;
        };
        match apply(&rom, &patch) {
            Ok(patched) => return patched,
            Err(error) => {
                eprintln!("ignoring {}: {error}", path.display());
            }
        }
    }
    rom
}

type ApplyFn = fn(&[u8], &[u8]) -> Result<Vec<u8>, PatchError>;

// --- IPS -----------------------------------------------------------------

pub fn apply_ips(source: &[u8], patch: &[u8]) -> Result<Vec<u8>, PatchError> {
    let mut rest = patch
        .strip_prefix(b"PATCH")
        .ok_or(PatchError::Malformed("missing PATCH magic"))?;
    let mut target = source.to_vec();

    loop {
        let (header, tail) = split(rest, 3)?;
        if header == b"EOF" {
            // An optional 3-byte length after EOF truncates the target.
            if let Ok((truncate, _)) = split(tail, 3) {
                target.truncate(be24(truncate));
            }
            return Ok(target);
        }
        let offset = be24(header);
        let (size, tail) = split(tail, 2)?;
        let size = u16::from_be_bytes([size[0], size[1]]) as usize;

        let (data, tail) = if size == 0 {
            // RLE record: repeat one byte.
            let (rle, tail) = split(tail, 3)?;
            let count = u16::from_be_bytes([rle[0], rle[1]]) as usize;
            (vec![rle[2]; count], tail)
        } else {
            let (data, tail) = split(tail, size)?;
            (data.to_vec(), tail)
        };

        if target.len() < offset + data.len() {
            target.resize(offset + data.len(), 0);
        }
        target[offset..offset + data.len()].copy_from_slice(&data);
        rest = tail;
    }
}

fn be24(bytes: &[u8]) -> usize {
    ((bytes[0] as usize) << 16) | ((bytes[1] as usize) << 8) | bytes[2] as usize
}

fn split(bytes: &[u8], n: usize) -> Result<(&[u8], &[u8]), PatchError> {
    if bytes.len() < n {
        return Err(PatchError::Malformed("truncated"));
    }
    Ok(bytes.split_at(n))
}

// --- BPS -----------------------------------------------------------------

/// The source CRC32 a BPS patch expects, read from its footer — how a
/// catalogue matches a patch to a base ROM without applying it. Awaits the
/// hack-catalogue integration.
#[allow(dead_code)]
pub fn bps_source_crc32(patch: &[u8]) -> Option<u32> {
    if patch.len() < 16 || !patch.starts_with(b"BPS1") {
        return None;
    }
    let footer = &patch[patch.len() - 12..];
    Some(u32::from_le_bytes(footer[0..4].try_into().unwrap()))
}

pub fn apply_bps(source: &[u8], patch: &[u8]) -> Result<Vec<u8>, PatchError> {
    if patch.len() < 4 + 12 {
        return Err(PatchError::Malformed("too short"));
    }
    let footer = &patch[patch.len() - 12..];
    let source_crc = u32::from_le_bytes(footer[0..4].try_into().unwrap());
    let target_crc = u32::from_le_bytes(footer[4..8].try_into().unwrap());
    let patch_crc = u32::from_le_bytes(footer[8..12].try_into().unwrap());

    if crc32(&patch[..patch.len() - 4]) != patch_crc {
        return Err(PatchError::Malformed("patch checksum"));
    }
    if crc32(source) != source_crc {
        return Err(PatchError::SourceMismatch);
    }

    let mut reader = BpsReader {
        bytes: &patch[4..patch.len() - 12],
        pos: 0,
    };
    if !patch.starts_with(b"BPS1") {
        return Err(PatchError::Malformed("missing BPS1 magic"));
    }

    let source_size = reader.number()? as usize;
    let target_size = reader.number()? as usize;
    let metadata_size = reader.number()? as usize;
    reader.skip(metadata_size)?;
    if source_size != source.len() {
        return Err(PatchError::SourceMismatch);
    }

    let mut target = Vec::with_capacity(target_size);
    let mut source_offset = 0usize;
    let mut target_offset = 0usize;

    while !reader.done() {
        let action = reader.number()? as usize;
        let length = (action >> 2) + 1;
        match action & 3 {
            0 => {
                // SourceRead: mirror the source at the write position.
                let start = target.len();
                let end = start + length;
                if end > source.len() {
                    return Err(PatchError::Malformed("SourceRead past source"));
                }
                target.extend_from_slice(&source[start..end]);
            }
            1 => {
                let data = reader.take(length)?;
                target.extend_from_slice(data);
            }
            2 => {
                source_offset = relative(source_offset, reader.number()?)?;
                let end = source_offset + length;
                if end > source.len() {
                    return Err(PatchError::Malformed("SourceCopy past source"));
                }
                target.extend_from_slice(&source[source_offset..end]);
                source_offset = end;
            }
            3 => {
                target_offset = relative(target_offset, reader.number()?)?;
                // Byte-by-byte: TargetCopy may overlap what it's writing.
                for _ in 0..length {
                    let byte = *target
                        .get(target_offset)
                        .ok_or(PatchError::Malformed("TargetCopy past target"))?;
                    target.push(byte);
                    target_offset += 1;
                }
            }
            _ => unreachable!(),
        }
    }

    if target.len() != target_size {
        return Err(PatchError::Malformed("target size"));
    }
    if crc32(&target) != target_crc {
        return Err(PatchError::Malformed("target checksum"));
    }
    Ok(target)
}

fn relative(base: usize, encoded: u64) -> Result<usize, PatchError> {
    let magnitude = (encoded >> 1) as usize;
    if encoded & 1 == 1 {
        base.checked_sub(magnitude)
            .ok_or(PatchError::Malformed("negative offset"))
    } else {
        Ok(base + magnitude)
    }
}

struct BpsReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl BpsReader<'_> {
    fn done(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn take(&mut self, n: usize) -> Result<&[u8], PatchError> {
        if self.pos + n > self.bytes.len() {
            return Err(PatchError::Malformed("truncated"));
        }
        let slice = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    fn skip(&mut self, n: usize) -> Result<(), PatchError> {
        self.take(n).map(|_| ())
    }

    /// BPS variable-width number: 7 data bits per byte, top bit terminates,
    /// each continuation implicitly adds the next shift step.
    fn number(&mut self) -> Result<u64, PatchError> {
        let mut data: u64 = 0;
        let mut shift: u64 = 1;
        loop {
            let [byte] = *self.take(1)? else {
                unreachable!()
            };
            data += (byte as u64 & 0x7f) * shift;
            if byte & 0x80 != 0 {
                return Ok(data);
            }
            shift <<= 7;
            data += shift;
        }
    }
}

// --- CRC32 (IEEE, as BPS uses) --------------------------------------------

pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_known_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn ips_applies_records_rle_and_truncation() {
        let source = vec![0u8; 8];
        let mut patch = b"PATCH".to_vec();
        patch.extend([0, 0, 2]); // offset 2
        patch.extend([0, 2]); // size 2
        patch.extend([0xaa, 0xbb]);
        patch.extend([0, 0, 5]); // offset 5
        patch.extend([0, 0]); // RLE marker
        patch.extend([0, 3, 0xcc]); // 3 × 0xcc — extends past source end
        patch.extend(b"EOF");
        patch.extend([0, 0, 7]); // truncate to 7

        let target = apply_ips(&source, &patch).unwrap();
        assert_eq!(target, [0, 0, 0xaa, 0xbb, 0, 0xcc, 0xcc]);
    }

    #[test]
    fn ips_rejects_garbage() {
        assert_eq!(
            apply_ips(&[0u8; 4], b"NOTIPS"),
            Err(PatchError::Malformed("missing PATCH magic"))
        );
    }

    fn bps_number(mut value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let x = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                out.push(0x80 | x);
                return out;
            }
            out.push(x);
            value -= 1;
        }
    }

    fn bps_patch(source: &[u8], target: &[u8], actions: &[u8]) -> Vec<u8> {
        let mut patch = b"BPS1".to_vec();
        patch.extend(bps_number(source.len() as u64));
        patch.extend(bps_number(target.len() as u64));
        patch.extend(bps_number(0)); // no metadata
        patch.extend(actions);
        patch.extend(crc32(source).to_le_bytes());
        patch.extend(crc32(target).to_le_bytes());
        patch.extend(crc32(&patch).to_le_bytes());
        patch
    }

    #[test]
    fn bps_round_trips_and_verifies() {
        let source = b"HELLO WORLD!".to_vec();
        let target = b"HELLO GBWORLGBW".to_vec();

        let mut actions = Vec::new();
        actions.extend(bps_number((6u64 - 1) << 2)); // SourceRead 6
        actions.extend(bps_number(((2u64 - 1) << 2) | 1)); // TargetRead 2
        actions.extend(b"GB");
        actions.extend(bps_number(((4u64 - 1) << 2) | 2)); // SourceCopy 4...
        actions.extend(bps_number(6 << 1)); // ...from source offset +6
        actions.extend(bps_number(((3u64 - 1) << 2) | 3)); // TargetCopy 3...
        actions.extend(bps_number(6 << 1)); // ...from target offset +6

        let patch = bps_patch(&source, &target, &actions);
        assert_eq!(apply_bps(&source, &patch).unwrap(), target);
    }

    #[test]
    fn bps_rejects_wrong_source() {
        let source = b"HELLO WORLD!".to_vec();
        let target = b"HELLO WORLD?".to_vec();
        let mut actions = Vec::new();
        actions.extend(bps_number((11u64 - 1) << 2)); // SourceRead 11
        actions.extend(bps_number(((1u64 - 1) << 2) | 1)); // TargetRead 1
        actions.extend(b"?");
        let patch = bps_patch(&source, &target, &actions);

        assert_eq!(apply_bps(&source, &patch).unwrap(), target);
        assert_eq!(
            apply_bps(b"DIFFERENT ROM", &patch),
            Err(PatchError::SourceMismatch)
        );
        assert_eq!(bps_source_crc32(&patch), Some(crc32(&source)));
    }

    #[test]
    fn bps_target_copy_can_overlap_itself() {
        let source = b"A".to_vec();
        // TargetRead "XY", then TargetCopy 6 from offset 0 — reads bytes it
        // is writing (classic RLE-by-overlap).
        let target = b"XYXYXYXY".to_vec();
        let mut actions = Vec::new();
        actions.extend(bps_number(((2u64 - 1) << 2) | 1));
        actions.extend(b"XY");
        actions.extend(bps_number(((6u64 - 1) << 2) | 3));
        actions.extend(bps_number(0)); // target offset +0
        let patch = bps_patch(&source, &target, &actions);
        assert_eq!(apply_bps(&source, &patch).unwrap(), target);
    }
}
