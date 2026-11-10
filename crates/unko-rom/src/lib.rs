//! ROM discovery and container reading.
//!
//! The library is backed by files on disk, but ROMs are commonly distributed
//! one-per-archive. This crate treats a ROM inside `.zip`, `.7z` or `.rar` the
//! same as a loose file: scanning produces one [`RomEntry`] per playable image,
//! and [`load`] returns the bytes.
//!
//! Design rules:
//! - scanning is best-effort and never aborts the whole library for one bad file
//! - ZIP headers are parsed during the scan (cheap first-block read)
//! - 7z/RAR headers are parsed on load, because their first-block read is not
//!   cheap in solid archives
//! - archive paths and inner names form the stable save id; the ROM is never
//!   extracted to disk

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use zip::ZipArchive;

/// Archive container a ROM came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    /// A loose file.
    File,
    /// A ZIP archive.
    Zip,
    /// A 7-Zip archive.
    SevenZ,
    /// A RAR archive.
    Rar,
}

impl Container {
    /// Short label for the UI.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Zip => "zip",
            Self::SevenZ => "7z",
            Self::Rar => "rar",
        }
    }
}

/// Where a ROM lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A plain file.
    File(PathBuf),
    /// A member of an archive.
    Archive {
        /// Archive path.
        archive: PathBuf,
        /// Member name inside the archive.
        inner: String,
        /// Member index, when the container supports direct indexing.
        index: Option<usize>,
    },
}

impl Source {
    /// Path shown to the user.
    #[must_use]
    pub fn display_path(&self) -> String {
        match self {
            Self::File(path) => path.display().to_string(),
            Self::Archive { archive, inner, .. } => {
                format!("{}/{}", archive.display(), inner)
            }
        }
    }

    /// Container of this source.
    #[must_use]
    pub fn container(self) -> Container {
        match self {
            Self::File(_) => Container::File,
            Self::Archive { archive, .. } => match extension(&archive).as_str() {
                "7z" => Container::SevenZ,
                "rar" => Container::Rar,
                _ => Container::Zip,
            },
        }
    }
}

/// Subset of the iNES / NES 2.0 header the library and UI care about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Mapper number, including the NES 2.0 high bits.
    pub mapper: u16,
    /// NES 2.0 submapper, 0 for iNES.
    pub submapper: u8,
    /// PRG-ROM size in KiB.
    pub prg_kib: u32,
    /// CHR-ROM size in KiB (0 means CHR-RAM).
    pub chr_kib: u32,
    /// Battery-backed save RAM present.
    pub battery: bool,
    /// Four-screen nametable wiring.
    pub four_screen: bool,
    /// NES 2.0 header.
    pub nes2: bool,
    /// Header timing hint.
    pub region: &'static str,
}

/// One playable image in the library.
#[derive(Debug, Clone)]
pub struct RomEntry {
    /// Where the bytes live.
    pub source: Source,
    /// Container label.
    pub container: Container,
    /// Display title, derived from the member/file name.
    pub title: String,
    /// File name (member name for archives).
    pub file_name: String,
    /// Uncompressed size in bytes.
    pub size: u64,
    /// Archive/file modification time, seconds since the Unix epoch.
    pub modified: Option<u64>,
    /// Parsed header, when cheaply available.
    pub header: Option<Header>,
    /// Stable id used for save names.
    pub id: u32,
}

impl RomEntry {
    /// System tag for the library column.
    #[must_use]
    pub const fn system(&self) -> &'static str {
        "nes"
    }

    /// Human-readable size.
    #[must_use]
    pub fn size_label(&self) -> String {
        let kib = self.size as f64 / 1024.0;
        if kib >= 1024.0 {
            format!("{:.1} MiB", kib / 1024.0)
        } else {
            format!("{kib:.0} KiB")
        }
    }

    /// Mapper label for the list / status bar.
    #[must_use]
    pub fn mapper_label(&self) -> String {
        match self.header {
            Some(header) => {
                if header.submapper > 0 {
                    format!("{}:{}", header.mapper, header.submapper)
                } else {
                    header.mapper.to_string()
                }
            }
            None => "—".into(),
        }
    }

    /// `NTSC`, `PAL`, `Dual`, or `—`.
    #[must_use]
    pub fn region_label(&self) -> &'static str {
        self.header.map_or("—", |header| header.region)
    }

    /// Stable file stem used for state/SRAM names.
    #[must_use]
    pub fn save_stem(&self) -> String {
        let clean: String = self
            .title
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ' ') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        format!("{clean}-{:08x}", self.id)
    }

    /// Archive name, if this entry came from one.
    #[must_use]
    pub fn archive_name(&self) -> Option<String> {
        match &self.source {
            Source::Archive { archive, .. } => archive
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            Source::File(_) => None,
        }
    }
}

/// Scans `root` recursively for loose ROMs and supported archives.
///
/// Headerless images are skipped when the container lets us inspect the header
/// cheaply (loose files and ZIP). 7z/RAR members are listed before decoding and
/// are validated on load.
#[must_use]
pub fn scan(root: &Path) -> Vec<RomEntry> {
    let mut entries = Vec::new();
    walk(root, 0, &mut entries);
    entries.sort_by_key(|entry| entry.title.to_lowercase());
    entries
}

/// Resolves a path to one or more entries.
///
/// A loose ROM yields one entry; an archive yields every `.nes` member.
pub fn probe_path(path: &Path) -> Result<Vec<RomEntry>, RomError> {
    if path.is_dir() {
        return Ok(scan(path));
    }
    if !path.is_file() {
        return Err(RomError::Missing(path.display().to_string()));
    }
    let extension = extension(path);
    match extension.as_str() {
        "nes" => {
            let entry = plain_entry(path.to_path_buf()).ok_or_else(|| {
                RomError::Invalid(format!("{} is not a valid iNES image", path.display()))
            })?;
            Ok(vec![entry])
        }
        "zip" | "7z" | "rar" => Ok(scan_archive(path)),
        other => Err(RomError::Unsupported(format!(
            "{} (.{other}) is not a supported ROM or archive",
            path.display()
        ))),
    }
}

/// Reads the bytes for an entry.
pub fn load(entry: &RomEntry) -> Result<Vec<u8>, RomError> {
    match &entry.source {
        Source::File(path) => fs::read(path).map_err(RomError::Io),
        Source::Archive {
            archive,
            inner,
            index,
        } => match entry.container {
            Container::Zip => load_zip(archive, inner, *index),
            Container::SevenZ => load_7z(archive, inner),
            Container::Rar => load_rar(archive, inner, *index),
            Container::File => Err(RomError::Invalid("archive source marked as file".into())),
        },
    }
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<RomEntry>) {
    if depth > 6 {
        return;
    }
    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            let hidden = entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with('.'));
            if !hidden {
                walk(&path, depth + 1, out);
            }
        } else if file_type.is_file() {
            match extension(&path).as_str() {
                "nes" => {
                    if let Some(entry) = plain_entry(path) {
                        out.push(entry);
                    }
                }
                "zip" | "7z" | "rar" => out.extend(scan_archive(&path)),
                _ => {}
            }
        }
    }
}

fn plain_entry(path: PathBuf) -> Option<RomEntry> {
    let bytes = fs::read(&path).ok()?;
    let header = parse_header(&bytes)?;
    let metadata = fs::metadata(&path).ok();
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown.nes".into());
    let title = path
        .file_stem()
        .map(|name| name.to_string_lossy().replace('_', " "))
        .unwrap_or_else(|| "Unknown".into());
    Some(RomEntry {
        source: Source::File(path),
        container: Container::File,
        title,
        file_name,
        size: bytes.len() as u64,
        modified: metadata
            .and_then(|meta| meta.modified().ok())
            .and_then(system_time_secs),
        header: Some(header),
        id: crc32(&bytes),
    })
}

fn scan_archive(path: &Path) -> Vec<RomEntry> {
    match extension(path).as_str() {
        "zip" => scan_zip(path),
        "7z" => scan_7z(path),
        "rar" => scan_rar(path),
        _ => Vec::new(),
    }
}

fn scan_zip(path: &Path) -> Vec<RomEntry> {
    let Ok(file) = fs::File::open(path) else {
        return Vec::new();
    };
    let Ok(mut archive) = ZipArchive::new(file) else {
        return Vec::new();
    };
    let modified = fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(system_time_secs);
    let mut out = Vec::new();
    for index in 0..archive.len() {
        let Ok(mut member) = archive.by_index(index) else {
            continue;
        };
        let Ok(name) = member.name() else {
            continue;
        };
        let name = name.into_owned();
        if member.is_dir() || !extension(Path::new(&name)).eq("nes") {
            continue;
        }
        let mut prefix = [0u8; 16];
        let header = match member.read_exact(&mut prefix) {
            Ok(()) => parse_header(&prefix),
            Err(_) => None,
        };
        let Some(header) = header else {
            continue; // headerless ZIP images are not playable
        };
        let title = member_title(&name);
        let size = member.size();
        let id = archive_id(path, &name);
        out.push(RomEntry {
            source: Source::Archive {
                archive: path.to_path_buf(),
                inner: name.clone(),
                index: Some(index),
            },
            container: Container::Zip,
            title,
            file_name: name,
            size,
            modified,
            header: Some(header),
            id,
        });
    }
    out
}

fn scan_7z(path: &Path) -> Vec<RomEntry> {
    let Ok(reader) = sevenz_rust2::ArchiveReader::open(path, sevenz_rust2::Password::empty())
    else {
        return Vec::new();
    };
    let modified = fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(system_time_secs);
    reader
        .archive()
        .files
        .iter()
        .filter(|file| !file.is_directory() && extension(Path::new(file.name())).eq("nes"))
        .map(|file| {
            let inner = file.name().to_string();
            RomEntry {
                source: Source::Archive {
                    archive: path.to_path_buf(),
                    inner: inner.clone(),
                    index: None,
                },
                container: Container::SevenZ,
                title: member_title(&inner),
                file_name: inner.clone(),
                size: file.size(),
                modified,
                header: None,
                id: archive_id(path, &inner),
            }
        })
        .collect()
}

fn scan_rar(path: &Path) -> Vec<RomEntry> {
    let Ok(archive) = rars::ArchiveReader::read_path(path) else {
        return Vec::new();
    };
    let members = archive.members();
    let modified = fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(system_time_secs);
    members
        .enumerate()
        .filter(|(_, member)| {
            !member.meta.is_directory && extension(Path::new(&member_name(member))).eq("nes")
        })
        .map(|(index, member)| {
            let inner = member_name(&member);
            RomEntry {
                source: Source::Archive {
                    archive: path.to_path_buf(),
                    inner: inner.clone(),
                    index: Some(index),
                },
                container: Container::Rar,
                title: member_title(&inner),
                file_name: inner.clone(),
                size: member.meta.unpacked_size,
                modified,
                header: None,
                id: archive_id(path, &inner),
            }
        })
        .collect()
}

fn load_zip(path: &Path, inner: &str, _index: Option<usize>) -> Result<Vec<u8>, RomError> {
    let file = fs::File::open(path).map_err(RomError::Io)?;
    let mut archive = ZipArchive::new(file).map_err(|err| RomError::Archive(err.to_string()))?;
    let mut member = archive
        .by_name(inner)
        .map_err(|err| RomError::Archive(err.to_string()))?;
    let mut bytes = Vec::with_capacity(member.size() as usize);
    member
        .read_to_end(&mut bytes)
        .map_err(|err| RomError::Archive(err.to_string()))?;
    Ok(bytes)
}

fn load_7z(path: &Path, inner: &str) -> Result<Vec<u8>, RomError> {
    let mut reader = sevenz_rust2::ArchiveReader::open(path, sevenz_rust2::Password::empty())
        .map_err(|err| RomError::Archive(err.to_string()))?;
    reader
        .read_file(inner)
        .map_err(|err| RomError::Archive(err.to_string()))
}

fn load_rar(path: &Path, inner: &str, index: Option<usize>) -> Result<Vec<u8>, RomError> {
    let archive =
        rars::ArchiveReader::read_path(path).map_err(|err| RomError::Archive(err.to_string()))?;
    let bytes = match index {
        Some(index) => archive.read_member_at(index, None),
        None => archive.read_member(inner.as_bytes(), None),
    }
    .map_err(|err| RomError::Archive(err.to_string()))?;
    bytes.ok_or_else(|| RomError::NotFound(inner.to_string()))
}

fn member_name(member: &rars::ArchiveMember) -> String {
    String::from_utf8_lossy(&member.meta.name).into_owned()
}

fn member_title(inner: &str) -> String {
    Path::new(inner)
        .file_stem()
        .map(|name| name.to_string_lossy().replace('_', " "))
        .unwrap_or_else(|| inner.to_string())
}

fn archive_id(path: &Path, inner: &str) -> u32 {
    let mut key = path.to_string_lossy().into_owned();
    key.push('\0');
    key.push_str(inner);
    crc32(key.as_bytes())
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn system_time_secs(time: SystemTime) -> Option<u64> {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}

/// Parses an iNES or NES 2.0 header.
#[must_use]
pub fn parse_header(bytes: &[u8]) -> Option<Header> {
    if bytes.len() < 16 || &bytes[0..4] != b"NES\x1a" {
        return None;
    }
    let flags6 = bytes[6];
    let flags7 = bytes[7];
    let nes2 = flags7 & 0x0C == 0x08;
    let mut mapper = u16::from(flags7 & 0xF0) | u16::from(flags6 >> 4);
    if nes2 {
        mapper |= u16::from(bytes[8] & 0x0F) << 8;
    }
    let region = if nes2 {
        match bytes[12] & 0x03 {
            0 => "NTSC",
            1 => "PAL",
            _ => "Dual",
        }
    } else if bytes[9] & 0x01 == 0 {
        "NTSC"
    } else {
        "PAL"
    };
    Some(Header {
        mapper,
        submapper: if nes2 { bytes[8] >> 4 } else { 0 },
        prg_kib: u32::from(bytes[4]) * 16,
        chr_kib: u32::from(bytes[5]) * 8,
        battery: flags6 & 0x02 != 0,
        four_screen: flags6 & 0x08 != 0,
        nes2,
        region,
    })
}

/// CRC-32 (IEEE 802.3), used as a stable id.
#[must_use]
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// ROM library errors.
#[derive(Debug)]
pub enum RomError {
    /// Filesystem failure.
    Io(std::io::Error),
    /// Container parse/decode failure.
    Archive(String),
    /// No such file.
    Missing(String),
    /// No playable image.
    Invalid(String),
    /// Member disappeared.
    NotFound(String),
    /// Extension not handled.
    Unsupported(String),
}

impl fmt::Display for RomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::Archive(err) => write!(f, "archive error: {err}"),
            Self::Missing(path) => write!(f, "not found: {path}"),
            Self::Invalid(err) => write!(f, "{err}"),
            Self::NotFound(name) => write!(f, "member not found: {name}"),
            Self::Unsupported(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for RomError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn synthetic_nes() -> Vec<u8> {
        let mut rom = vec![0u8; 16 + 16 * 1024 + 8 * 1024];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = 1;
        rom[5] = 2;
        rom[6] = 0x12;
        rom[7] = 0x10;
        rom
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        std::env::temp_dir().join(format!("unko-rom-{tag}-{nanos}"))
    }

    #[test]
    fn parses_ines_header() {
        let header = parse_header(&synthetic_nes()).expect("header");
        assert_eq!(header.mapper, 17);
        assert_eq!(header.prg_kib, 16);
        assert_eq!(header.chr_kib, 16);
        assert!(header.battery);
        assert!(!header.nes2);
        assert_eq!(header.region, "NTSC");
    }

    #[test]
    fn crc_is_known_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn scans_loose_and_zipped_roms() {
        let dir = temp_dir("zip");
        fs::create_dir_all(&dir).expect("dir");
        let rom = synthetic_nes();
        fs::write(dir.join("loose.nes"), &rom).expect("loose");

        let zip_path = dir.join("pack.zip");
        let file = fs::File::create(&zip_path).expect("zip file");
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("packed.nes", options).expect("start");
        zip.write_all(&rom).expect("write member");
        zip.finish().expect("finish");

        let entries = scan(&dir);
        assert_eq!(entries.len(), 2, "loose + zipped");
        let zipped = entries
            .iter()
            .find(|entry| entry.container == Container::Zip)
            .expect("zip entry");
        assert_eq!(zipped.title, "packed");
        assert_eq!(zipped.header.expect("header").mapper, 17);
        let bytes = load(zipped).expect("loads member");
        assert_eq!(bytes, rom);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn headerless_zip_members_are_skipped() {
        let dir = temp_dir("headerless");
        fs::create_dir_all(&dir).expect("dir");
        let zip_path = dir.join("raw.zip");
        let file = fs::File::create(&zip_path).expect("zip file");
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("raw.nes", options).expect("start");
        zip.write_all(&[0u8; 64]).expect("write");
        zip.finish().expect("finish");
        assert!(scan(&dir).is_empty());
        let _ = fs::remove_dir_all(dir);
    }
}
