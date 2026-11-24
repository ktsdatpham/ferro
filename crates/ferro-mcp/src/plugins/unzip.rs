//! Unpacking a plugin `.zip` (user input): stored and deflated entries only,
//! no encryption, no zip64. Every entry path is checked so nothing lands
//! outside the destination folder ("zip slip": absolute paths, drive letters,
//! `..`), and the number of files and the unpacked size are limited (a small
//! zip may inflate to gigabytes).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

/// Limits for [`extract`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZipLimits {
    /// Most files unpacked.
    pub max_files: usize,
    /// Most bytes unpacked, all files together.
    pub max_bytes: u64,
}

const EOCD_SIG: u32 = 0x0605_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const LOCAL_SIG: u32 = 0x0403_4b50;

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// The relative path an entry name stands for, or `None` when it would
/// leave the destination (absolute, a drive letter, `..`) or is empty.
/// Backslashes count as separators (zips made on Windows).
pub fn safe_path(name: &str) -> Option<PathBuf> {
    let name = name.replace('\\', "/");
    if name.starts_with('/') || name.contains(':') || name.contains('\0') {
        return None;
    }
    let mut out = PathBuf::new();
    for c in Path::new(&name).components() {
        match c {
            Component::Normal(n) => out.push(n),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

struct Entry {
    name: String,
    method: u16,
    flags: u16,
    compressed: u64,
    size: u64,
    local_offset: u64,
}

fn bad(m: &str) -> String {
    format!("not a valid .zip file: {m}")
}

fn central_directory(f: &mut File) -> Result<Vec<Entry>, String> {
    let len = f.seek(SeekFrom::End(0)).map_err(|e| bad(&e.to_string()))?;
    let tail_len = len.min(22 + 65_535);
    f.seek(SeekFrom::Start(len - tail_len))
        .map_err(|e| bad(&e.to_string()))?;
    let mut tail = vec![0u8; tail_len as usize];
    f.read_exact(&mut tail).map_err(|e| bad(&e.to_string()))?;
    let eocd = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|&i| u32_at(&tail, i) == Some(EOCD_SIG))
        .ok_or_else(|| bad("no end of central directory"))?;
    let count = u16_at(&tail, eocd + 10).ok_or_else(|| bad("short header"))? as usize;
    let cd_size = u32_at(&tail, eocd + 12).ok_or_else(|| bad("short header"))? as u64;
    let cd_offset = u32_at(&tail, eocd + 16).ok_or_else(|| bad("short header"))? as u64;
    if cd_offset == 0xFFFF_FFFF || count == 0xFFFF {
        return Err(bad("zip64 archives are not supported"));
    }
    if cd_offset.saturating_add(cd_size) > len {
        return Err(bad("central directory outside the file"));
    }
    f.seek(SeekFrom::Start(cd_offset))
        .map_err(|e| bad(&e.to_string()))?;
    let mut cd = vec![0u8; cd_size as usize];
    f.read_exact(&mut cd).map_err(|e| bad(&e.to_string()))?;
    let mut entries = Vec::with_capacity(count.min(10_000));
    let mut i = 0usize;
    for _ in 0..count {
        if u32_at(&cd, i) != Some(CENTRAL_SIG) {
            return Err(bad("broken central directory"));
        }
        let field = |o: usize| u16_at(&cd, i + o).ok_or_else(|| bad("short entry"));
        let field32 = |o: usize| u32_at(&cd, i + o).ok_or_else(|| bad("short entry"));
        let flags = field(8)?;
        let method = field(10)?;
        let compressed = field32(20)? as u64;
        let size = field32(24)? as u64;
        let name_len = field(28)? as usize;
        let extra_len = field(30)? as usize;
        let comment_len = field(32)? as usize;
        let local_offset = field32(42)? as u64;
        let name_bytes = cd
            .get(i + 46..i + 46 + name_len)
            .ok_or_else(|| bad("short entry name"))?;
        entries.push(Entry {
            name: String::from_utf8_lossy(name_bytes).into_owned(),
            method,
            flags,
            compressed,
            size,
            local_offset,
        });
        i += 46 + name_len + extra_len + comment_len;
    }
    Ok(entries)
}

/// Unpack the zip at `zip` into `dest` (which must exist). Returns the
/// number of files written. Fails, possibly after writing some files (the
/// caller unpacks into a staging folder it removes), when the file is not a
/// zip this reader handles (zip64, encrypted, a method other than stored or
/// deflated), an entry path leaves `dest`, the limits are passed, or an
/// entry's data is broken.
pub fn extract(zip: &Path, dest: &Path, limits: &ZipLimits) -> Result<usize, String> {
    let mut f = File::open(zip).map_err(|e| format!("cannot open {}: {e}", zip.display()))?;
    let entries = central_directory(&mut f)?;
    let mut files = 0usize;
    let mut total = 0u64;
    for e in entries {
        let Some(rel) = safe_path(&e.name) else {
            return Err(format!(
                "the zip holds a path outside its folder (`{}`); refused",
                e.name
            ));
        };
        let out = dest.join(&rel);
        if e.name.ends_with('/') || e.name.ends_with('\\') {
            std::fs::create_dir_all(&out)
                .map_err(|err| format!("cannot create {}: {err}", out.display()))?;
            continue;
        }
        if e.flags & 1 != 0 {
            return Err("encrypted zips are not supported".into());
        }
        if e.compressed == 0xFFFF_FFFF || e.size == 0xFFFF_FFFF {
            return Err(bad("zip64 entries are not supported"));
        }
        files += 1;
        if files > limits.max_files {
            return Err(format!(
                "the zip holds more than {} files",
                limits.max_files
            ));
        }
        // Local header: its name and extra lengths can differ from the
        // central directory's.
        f.seek(SeekFrom::Start(e.local_offset))
            .map_err(|err| bad(&err.to_string()))?;
        let mut head = [0u8; 30];
        f.read_exact(&mut head)
            .map_err(|err| bad(&err.to_string()))?;
        if u32_at(&head, 0) != Some(LOCAL_SIG) {
            return Err(bad("broken local header"));
        }
        let skip = u16_at(&head, 26).unwrap_or(0) as i64 + u16_at(&head, 28).unwrap_or(0) as i64;
        f.seek(SeekFrom::Current(skip))
            .map_err(|err| bad(&err.to_string()))?;
        let room = limits.max_bytes.saturating_sub(total);
        let raw = (&mut f).take(e.compressed);
        let mut data = Vec::new();
        let read = match e.method {
            0 => raw.take(room + 1).read_to_end(&mut data),
            8 => flate2::read::DeflateDecoder::new(raw)
                .take(room + 1)
                .read_to_end(&mut data),
            m => {
                return Err(format!(
                    "zip method {m} is not supported (use stored or deflate)"
                ))
            }
        };
        read.map_err(|err| bad(&format!("{}: {err}", e.name)))?;
        if data.len() as u64 > room {
            return Err(format!(
                "the zip unpacks to more than {} MB",
                limits.max_bytes / (1024 * 1024)
            ));
        }
        if data.len() as u64 != e.size {
            return Err(bad(&format!("{}: wrong size", e.name)));
        }
        total += data.len() as u64;
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("cannot create {}: {err}", parent.display()))?;
        }
        let mut w =
            File::create(&out).map_err(|err| format!("cannot write {}: {err}", out.display()))?;
        w.write_all(&data)
            .map_err(|err| format!("cannot write {}: {err}", out.display()))?;
    }
    Ok(files)
}
