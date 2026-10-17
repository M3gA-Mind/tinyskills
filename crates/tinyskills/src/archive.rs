//! Reading an uploaded skill archive: a `.zip` (or `.skill`), `.tar` or
//! `.tar.gz` carrying one `SKILL.md`, at its root or inside one directory.
//!
//! This answers one question — what document and files did the archive carry —
//! and refuses everything it cannot answer that for. Whether the document is
//! acceptable, which slug it is stored under, and whether bundled files are
//! welcome are the host's decisions; [`crate::scan_skill`] and the validators
//! are the gates the result should pass next.
//!
//! # The archive is the attack surface
//!
//! An archive is a list of paths and byte counts supplied by whoever built it,
//! and every one is hostile input. Each entry is checked before its content is
//! read, so a bomb is refused by arithmetic rather than by running out of
//! memory:
//!
//! * an entry count ceiling ([`ArchiveLimits::max_entries`]);
//! * the running sum of declared sizes against [`ArchiveLimits::max_bytes`];
//! * absolute, drive-qualified, `..`-climbing and backslash-separated paths;
//! * symbolic and hard links, which are how an archive reaches a path it never
//!   names;
//! * an archive nested inside the archive.
//!
//! Declared sizes are the archive's own claim, so content is also read through
//! a bound, and a gzip stream is decompressed through one — a header that lies
//! becomes a refusal, not an allocation.
//!
//! macOS bookkeeping (`__MACOSX/`, `.DS_Store`, `._*` sidecars) is dropped
//! after the path checks, which still apply to it: Finder's Compress adds it
//! to every archive an operator on a Mac makes.

use std::io::Read;

use crate::model::SKILL_MD;

/// The default ceiling on entries an archive may declare.
pub const MAX_ARCHIVE_ENTRIES: usize = 64;

/// The default ceiling on what an archive may hold once expanded, in bytes.
pub const MAX_ARCHIVE_BYTES: u64 = 1024 * 1024;

/// The longest entry path accepted, in characters.
const MAX_ENTRY_PATH_CHARS: usize = 512;

/// Which reader an archive needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveFormat {
    /// A zip archive; `.skill` is a zip by another name.
    Zip,
    /// An uncompressed tar archive.
    Tar,
    /// A gzip-compressed tar archive.
    TarGz,
}

impl ArchiveFormat {
    /// The format a file name's extension names, case-insensitively: `.zip`
    /// and `.skill` are zip, `.tar` is tar, `.tar.gz` and `.tgz` are gzipped
    /// tar. `None` for anything else — chosen by name rather than by sniffing,
    /// so a caller can tell a user which formats it takes.
    #[must_use]
    // The comparisons run on a lowercased copy, which is the case-insensitive
    // check the lint asks for; `Path::extension` cannot see `.tar.gz`.
    #[allow(clippy::case_sensitive_file_extension_comparisons)]
    pub fn from_file_name(name: &str) -> Option<Self> {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".zip") || lower.ends_with(".skill") {
            Some(Self::Zip)
        } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
            Some(Self::TarGz)
        } else if lower.ends_with(".tar") {
            Some(Self::Tar)
        } else {
            None
        }
    }
}

/// How much an archive may hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArchiveLimits {
    /// The most entries, directories included.
    pub max_entries: usize,
    /// The most bytes once expanded, summed over every entry.
    pub max_bytes: u64,
}

impl Default for ArchiveLimits {
    fn default() -> Self {
        Self {
            max_entries: MAX_ARCHIVE_ENTRIES,
            max_bytes: MAX_ARCHIVE_BYTES,
        }
    }
}

/// A file bundled beside the archive's `SKILL.md`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveFile {
    /// The path relative to the skill directory (the archive's root
    /// directory, when it has one), `/`-separated.
    pub path: String,
    /// The file's content.
    pub bytes: Vec<u8>,
}

/// What a skill archive carried.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillArchive {
    /// The single top directory every file sat under, or `None` when the
    /// files sat at the archive's root. A host typically takes the skill's
    /// slug from it, after validating it as one.
    pub root: Option<String>,
    /// The `SKILL.md` source, verbatim.
    pub document: String,
    /// Every other file, in archive order.
    pub resources: Vec<ArchiveFile>,
}

/// Why an archive was refused. Each message is a sentence a user can read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ArchiveError {
    /// The bytes are not a readable archive of the stated format.
    #[error("that archive could not be read: {0}")]
    Unreadable(String),
    /// More entries than [`ArchiveLimits::max_entries`].
    #[error("that archive holds more than {max} entries")]
    TooManyEntries {
        /// The ceiling it exceeded.
        max: usize,
    },
    /// More bytes than [`ArchiveLimits::max_bytes`], declared or read.
    #[error("that archive expands to more than {max_bytes} bytes")]
    TooLarge {
        /// The ceiling it exceeded.
        max_bytes: u64,
    },
    /// An entry with an empty name.
    #[error("that archive holds an entry with no name")]
    EmptyPath,
    /// An entry whose path separates directories with `\`.
    #[error("`{0}` in that archive is not a relative path; entries separate directories with `/`")]
    BackslashPath(String),
    /// An entry whose path is absolute or drive-qualified.
    #[error("`{0}` in that archive is an absolute path")]
    AbsolutePath(String),
    /// An entry whose path climbs out of the archive with `..`.
    #[error("`{0}` in that archive points outside it")]
    Traversal(String),
    /// An entry whose path is unreasonably long.
    #[error("that archive holds an entry with an unreasonably long path")]
    PathTooLong,
    /// A symbolic or hard link.
    #[error("`{0}` in that archive is a link; a skill archive is read as plain files")]
    Link(String),
    /// An archive inside the archive.
    #[error("`{0}` in that archive is itself an archive; a skill archive is read one level deep")]
    NestedArchive(String),
    /// Files under two top directories, or at the root beside a directory.
    #[error(
        "that archive has no single skill directory; put `SKILL.md` at its root or inside one directory"
    )]
    MultipleRoots,
    /// No `SKILL.md` where one belongs.
    #[error("that archive has no `SKILL.md` at its root or inside its one directory")]
    NoSkillDocument,
    /// The `SKILL.md` is not UTF-8 text.
    #[error("that archive's `SKILL.md` is not UTF-8 text")]
    NotUtf8,
}

/// Reads a skill archive.
///
/// # Errors
///
/// An [`ArchiveError`] naming the first thing that made the archive
/// unacceptable; nothing is returned from a refused archive.
pub fn read_skill_archive(
    format: ArchiveFormat,
    bytes: &[u8],
    limits: &ArchiveLimits,
) -> Result<SkillArchive, ArchiveError> {
    let files = match format {
        ArchiveFormat::Zip => read_zip(bytes, limits)?,
        ArchiveFormat::Tar => read_tar(bytes, limits)?,
        ArchiveFormat::TarGz => {
            // Bounded so a gzip bomb stops at the ceiling (plus tar's own
            // header and padding blocks) instead of inflating without end.
            let entry_overhead = (limits.max_entries as u64 + 2).saturating_mul(1024);
            let bound = limits.max_bytes.saturating_add(entry_overhead);
            let decoder = flate2::read::GzDecoder::new(bytes).take(bound);
            read_tar(decoder, limits)?
        }
    };
    assemble(files)
}

/// One accepted file entry, in archive order.
struct Entry {
    path: String,
    bytes: Vec<u8>,
}

/// Tracks the entry count and declared bytes as an archive is walked.
struct Budget<'a> {
    limits: &'a ArchiveLimits,
    entries: usize,
    bytes: u64,
}

impl Budget<'_> {
    fn admit(&mut self, declared: u64) -> Result<(), ArchiveError> {
        self.entries += 1;
        if self.entries > self.limits.max_entries {
            return Err(ArchiveError::TooManyEntries {
                max: self.limits.max_entries,
            });
        }
        self.bytes = self.bytes.saturating_add(declared);
        if self.bytes > self.limits.max_bytes {
            return Err(ArchiveError::TooLarge {
                max_bytes: self.limits.max_bytes,
            });
        }
        Ok(())
    }
}

fn read_zip(bytes: &[u8], limits: &ArchiveLimits) -> Result<Vec<Entry>, ArchiveError> {
    let unreadable = |error: zip::result::ZipError| ArchiveError::Unreadable(error.to_string());
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(unreadable)?;
    if archive.len() > limits.max_entries {
        return Err(ArchiveError::TooManyEntries {
            max: limits.max_entries,
        });
    }

    // The shape pass reads only the central directory: nothing is
    // decompressed until every entry has been checked.
    let mut budget = Budget {
        limits,
        entries: 0,
        bytes: 0,
    };
    let mut wanted = Vec::new();
    for index in 0..archive.len() {
        let entry = archive.by_index_raw(index).map_err(unreadable)?;
        let path = entry.name().map_err(unreadable)?.into_owned();
        check_entry_path(&path)?;
        let path = normalize_entry_path(&path);
        if path.is_empty() {
            continue;
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170_000 == 0o120_000)
        {
            return Err(ArchiveError::Link(path.to_string()));
        }
        budget.admit(entry.size())?;
        if entry.is_dir() || keep_file(path)?.is_none() {
            continue;
        }
        wanted.push(index);
    }

    let mut files = Vec::with_capacity(wanted.len());
    let mut remaining = limits.max_bytes;
    for index in wanted {
        let entry = archive.by_index(index).map_err(unreadable)?;
        let path = normalize_entry_path(&entry.name().map_err(unreadable)?).to_string();
        let bytes = read_bounded(entry, &mut remaining, limits.max_bytes)?;
        files.push(Entry { path, bytes });
    }
    Ok(files)
}

fn read_tar(reader: impl Read, limits: &ArchiveLimits) -> Result<Vec<Entry>, ArchiveError> {
    let unreadable = |error: std::io::Error| ArchiveError::Unreadable(error.to_string());
    let mut archive = tar::Archive::new(reader);
    let mut budget = Budget {
        limits,
        entries: 0,
        bytes: 0,
    };
    let mut remaining = limits.max_bytes;
    let mut files = Vec::new();
    for entry in archive.entries().map_err(unreadable)? {
        let entry = entry.map_err(unreadable)?;
        let kind = entry.header().entry_type();
        if matches!(
            kind,
            tar::EntryType::XGlobalHeader | tar::EntryType::XHeader
        ) {
            continue;
        }
        let path = String::from_utf8(entry.path_bytes().into_owned())
            .map_err(|_| ArchiveError::Unreadable("an entry path is not UTF-8".to_string()))?;
        check_entry_path(&path)?;
        let path = normalize_entry_path(&path).to_string();
        if path.is_empty() {
            continue;
        }
        if kind.is_symlink() || kind.is_hard_link() {
            return Err(ArchiveError::Link(path));
        }
        budget.admit(entry.header().size().map_err(unreadable)?)?;
        if kind.is_dir() {
            continue;
        }
        if !kind.is_file() {
            return Err(ArchiveError::Unreadable(format!(
                "`{path}` is neither a file nor a directory"
            )));
        }
        if keep_file(&path)?.is_none() {
            continue;
        }
        let bytes = read_bounded(entry, &mut remaining, limits.max_bytes)?;
        files.push(Entry { path, bytes });
    }
    Ok(files)
}

/// Reads one entry's content through what is left of the byte budget.
fn read_bounded(
    reader: impl Read,
    remaining: &mut u64,
    max_bytes: u64,
) -> Result<Vec<u8>, ArchiveError> {
    let mut buffer = Vec::new();
    reader
        .take(remaining.saturating_add(1))
        .read_to_end(&mut buffer)
        .map_err(|error| ArchiveError::Unreadable(error.to_string()))?;
    let read = buffer.len() as u64;
    if read > *remaining {
        return Err(ArchiveError::TooLarge { max_bytes });
    }
    *remaining -= read;
    Ok(buffer)
}

/// `Some(())` for a file entry worth keeping, `None` for macOS bookkeeping.
///
/// # Errors
///
/// [`ArchiveError::NestedArchive`] for an archive inside the archive.
fn keep_file(path: &str) -> Result<Option<()>, ArchiveError> {
    const NESTED: [&str; 9] = [
        ".zip", ".skill", ".tar", ".gz", ".tgz", ".bz2", ".xz", ".7z", ".rar",
    ];
    let lower = path.to_ascii_lowercase();
    if NESTED.iter().any(|suffix| lower.ends_with(suffix)) {
        return Err(ArchiveError::NestedArchive(path.to_string()));
    }
    let mut segments = path.split('/');
    let mac = segments.clone().any(|segment| segment == "__MACOSX")
        || segments
            .next_back()
            .is_some_and(|name| name == ".DS_Store" || name.starts_with("._"));
    Ok((!mac).then_some(()))
}

/// Refuses a path that escapes the archive, naming the form it took.
fn check_entry_path(path: &str) -> Result<(), ArchiveError> {
    if path.is_empty() {
        return Err(ArchiveError::EmptyPath);
    }
    if path.contains('\\') {
        return Err(ArchiveError::BackslashPath(path.to_string()));
    }
    let mut chars = path.chars();
    let drive =
        matches!((chars.next(), chars.next()), (Some(c), Some(':')) if c.is_ascii_alphabetic());
    if path.starts_with('/') || drive {
        return Err(ArchiveError::AbsolutePath(path.to_string()));
    }
    if path.split('/').any(|part| part == "..") {
        return Err(ArchiveError::Traversal(path.to_string()));
    }
    if path.chars().count() > MAX_ENTRY_PATH_CHARS {
        return Err(ArchiveError::PathTooLong);
    }
    Ok(())
}

/// Removes harmless leading `./` components from archive entry paths.
fn normalize_entry_path(path: &str) -> &str {
    let path = path.trim_start_matches("./");
    if path == "." { "" } else { path }
}

/// Finds the one skill directory and splits its document from its resources.
fn assemble(files: Vec<Entry>) -> Result<SkillArchive, ArchiveError> {
    let mut roots: Vec<&str> = Vec::new();
    let mut at_root = false;
    for file in &files {
        match file.path.split_once('/') {
            Some((head, _)) if !roots.contains(&head) => roots.push(head),
            Some(_) => {}
            None => at_root = true,
        }
    }
    let root = match (at_root, roots.as_slice()) {
        (_, []) => None,
        (false, [only]) => Some((*only).to_string()),
        _ => return Err(ArchiveError::MultipleRoots),
    };
    let prefix = root
        .as_ref()
        .map(|dir| format!("{dir}/"))
        .unwrap_or_default();

    let mut document = None;
    let mut resources = Vec::new();
    for file in files {
        let relative = file.path[prefix.len()..].to_string();
        if relative == SKILL_MD {
            document = Some(String::from_utf8(file.bytes).map_err(|_| ArchiveError::NotUtf8)?);
        } else {
            resources.push(ArchiveFile {
                path: relative,
                bytes: file.bytes,
            });
        }
    }
    Ok(SkillArchive {
        root,
        document: document.ok_or(ArchiveError::NoSkillDocument)?,
        resources,
    })
}

#[cfg(test)]
#[path = "archive_tests.rs"]
mod tests;
