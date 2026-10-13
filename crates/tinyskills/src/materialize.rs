//! Rebuilding a skill tree on disk from a host's resolved skill set.
//!
//! A host that resolves which skills an agent has (from bundles on disk,
//! stored documents, or both) writes the result into a `<root>/<dir>/` tree
//! that discovery and the resource readers then scan. [`materialize_tree`]
//! rebuilds that tree from nothing on every call, so a skill dropped from the
//! set disappears from disk.
//!
//! The function takes open directory handles ([`cap_std::fs::Dir`]) and no
//! paths, so it resolves no path itself. Opening the destination's parent and
//! each source directory is the caller's job, and whatever the caller's open
//! resolved is what the function works on.

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, File, OpenOptions};

use crate::catalog::is_safe_segment;
use crate::model::SKILL_MD;

/// The deepest directory nesting [`materialize_tree`] copies below one
/// entry's directory.
pub const MAX_MATERIALIZE_DEPTH: usize = 32;

/// The largest single file [`materialize_tree`] copies out of a bundle
/// directory.
pub const MAX_MATERIALIZE_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// Where one materialized skill's content comes from.
#[derive(Clone, Debug)]
pub enum MaterializeSource {
    /// An open bundle directory, copied recursively: regular files and
    /// directories only, symlinks skipped.
    Dir(Arc<Dir>),
    /// One `SKILL.md` document, written as the only file.
    Document(String),
    /// A bundle already read into memory (for example from a blob store),
    /// written at its relative paths. Each path must be `/`-separated safe
    /// segments, no deeper than [`MAX_MATERIALIZE_DEPTH`], and each file no
    /// larger than [`MAX_MATERIALIZE_FILE_BYTES`].
    Files(Vec<MaterializeFile>),
}

/// One file of a [`MaterializeSource::Files`] bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterializeFile {
    /// The `/`-separated path inside the skill's directory.
    pub path: String,
    /// The file contents.
    pub bytes: Vec<u8>,
}

impl MaterializeFile {
    /// A file at `path` with `bytes`.
    #[must_use]
    pub fn new(path: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            path: path.into(),
            bytes: bytes.into(),
        }
    }
}

/// One skill to write under the tree's root.
#[derive(Clone, Debug)]
pub struct MaterializeEntry {
    /// The directory name under the root; must pass
    /// [`is_safe_segment`](crate::is_safe_segment).
    pub dir_name: String,
    /// The skill's content.
    pub source: MaterializeSource,
}

/// What [`materialize_tree`] wrote.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MaterializeReport {
    /// Each entry's directory name, in the order written.
    pub dirs: Vec<String>,
    /// Regular files written or copied.
    pub files: usize,
    /// Symlinks found in source directories and not followed.
    pub skipped_symlinks: usize,
}

/// Why [`materialize_tree`] failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MaterializeError {
    /// The root name is not a single safe path segment.
    #[error("`{root_name}` is not a safe skill tree name")]
    UnsafeRootName {
        /// The refused name.
        root_name: String,
    },
    /// An entry's directory name is not a single safe path segment.
    #[error("`{dir_name}` is not a safe skill directory name")]
    UnsafeDirName {
        /// The refused name.
        dir_name: String,
    },
    /// Two entries share a directory name.
    #[error("two skills would both be written to `{dir_name}`")]
    DuplicateDirName {
        /// The shared name.
        dir_name: String,
    },
    /// A [`MaterializeSource::Files`] path is not `/`-separated safe segments.
    #[error("`{path}` is not a safe bundle file path")]
    UnsafeFilePath {
        /// The refused path.
        path: String,
    },
    /// A source directory nests deeper than [`MAX_MATERIALIZE_DEPTH`].
    #[error("{path} nests more than {max} directories deep")]
    TooDeep {
        /// The first directory past the limit, relative to its entry's
        /// directory name.
        path: PathBuf,
        /// The limit.
        max: usize,
    },
    /// A source file is larger than [`MAX_MATERIALIZE_FILE_BYTES`].
    #[error("{path} is larger than {max} bytes")]
    FileTooLarge {
        /// The oversized file, relative to its entry's directory name.
        path: PathBuf,
        /// The limit.
        max: u64,
    },
    /// A filesystem operation failed.
    #[error("{context} {path}: {source}")]
    Io {
        /// The operation that failed.
        context: &'static str,
        /// The path involved, relative to the tree's root.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
}

/// Replaces `root_name` inside `parent` with one directory per entry.
///
/// `root_name` and every `dir_name` are checked before anything on disk
/// changes. The new tree is written beside the old one and swapped in only
/// once every entry has been written, so a failure leaves the previous tree
/// in place and a source that lives inside the old tree is read before the
/// old tree is removed. Each entry is written to `root_name/<dir_name>/`: a
/// [`MaterializeSource::Document`] as its `SKILL.md`, a
/// [`MaterializeSource::Dir`] as a recursive copy that skips symlinks and
/// other non-regular files and refuses files over
/// [`MAX_MATERIALIZE_FILE_BYTES`].
///
/// Every operation is made relative to an open handle and never follows a
/// symlink: a source entry swapped for a symlink while the copy runs fails the
/// copy, and a symlink planted at `root_name` is removed, not followed. The
/// function resolves no path, so the guarantee covers everything at or below
/// `parent` and the source handles. How those handles were opened, including
/// any symlink among the ancestors of the paths they came from, is the
/// caller's. A directory replaced by another real directory by someone who can
/// write to `parent` or a source cannot be told apart from the original.
///
/// `parent` must not lie inside a source directory: the new tree would be
/// copied into itself until [`MAX_MATERIALIZE_DEPTH`] is exceeded.
///
/// ```
/// use std::sync::Arc;
/// use tinyskills::cap_fs_ext::DirExt;
/// use tinyskills::cap_std::{ambient_authority, fs::Dir};
/// use tinyskills::{MaterializeEntry, MaterializeSource, materialize_tree};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let temp = tempfile::tempdir()?;
/// std::fs::create_dir(temp.path().join("bundle"))?;
/// std::fs::write(temp.path().join("bundle/SKILL.md"), "---\nname: A\ndescription: B\n---\n")?;
///
/// let base = Dir::open_ambient_dir(temp.path(), ambient_authority())?;
/// let bundle = base.open_dir_nofollow("bundle")?;
/// let entries = [MaterializeEntry {
///     dir_name: "a".to_string(),
///     source: MaterializeSource::Dir(Arc::new(bundle)),
/// }];
/// let report = materialize_tree(&base, "skills", &entries)?;
/// assert_eq!(report.files, 1);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// [`MaterializeError::UnsafeRootName`], [`MaterializeError::UnsafeDirName`]
/// or [`MaterializeError::DuplicateDirName`] before anything is touched;
/// [`MaterializeError::FileTooLarge`] when a source file exceeds the limit;
/// [`MaterializeError::TooDeep`] when a source nests past
/// [`MAX_MATERIALIZE_DEPTH`]; [`MaterializeError::Io`] when clearing,
/// creating, reading, or writing fails.
pub fn materialize_tree(
    parent: &Dir,
    root_name: &str,
    entries: &[MaterializeEntry],
) -> Result<MaterializeReport, MaterializeError> {
    if !is_safe_segment(root_name) {
        return Err(MaterializeError::UnsafeRootName {
            root_name: root_name.to_string(),
        });
    }
    let mut seen = BTreeSet::new();
    for entry in entries {
        if !is_safe_segment(&entry.dir_name) {
            return Err(MaterializeError::UnsafeDirName {
                dir_name: entry.dir_name.clone(),
            });
        }
        if !seen.insert(entry.dir_name.as_str()) {
            return Err(MaterializeError::DuplicateDirName {
                dir_name: entry.dir_name.clone(),
            });
        }
    }

    let staging = format!(".{root_name}.materializing");
    remove_entry(parent, &staging)?;
    let built = build(parent, &staging, entries);
    if built.is_err() {
        let _ = remove_entry(parent, &staging);
    }
    let report = built?;

    remove_entry(parent, root_name)?;
    at(
        "replacing",
        Path::new(root_name),
        parent.rename(&staging, parent, root_name),
    )?;
    Ok(report)
}

fn build(
    parent: &Dir,
    staging: &str,
    entries: &[MaterializeEntry],
) -> Result<MaterializeReport, MaterializeError> {
    let tree = at(
        "creating skill tree",
        Path::new(staging),
        create_dir(parent, OsStr::new(staging)),
    )?;
    let mut report = MaterializeReport::default();
    for entry in entries {
        let rel = Path::new(&entry.dir_name);
        let out = at(
            "creating",
            rel,
            create_dir(&tree, OsStr::new(&entry.dir_name)),
        )?;
        match &entry.source {
            MaterializeSource::Dir(source) => copy_dir(source, &out, rel, 0, &mut report)?,
            MaterializeSource::Files(files) => write_files(&out, rel, files, &mut report)?,
            MaterializeSource::Document(document) => {
                let file = rel.join(SKILL_MD);
                let mut handle = at("creating", &file, create_file(&out, OsStr::new(SKILL_MD)))?;
                at("writing", &file, handle.write_all(document.as_bytes()))?;
                report.files += 1;
            }
        }
        report.dirs.push(entry.dir_name.clone());
    }
    Ok(report)
}

fn write_files(
    out: &Dir,
    rel: &Path,
    files: &[MaterializeFile],
    report: &mut MaterializeReport,
) -> Result<(), MaterializeError> {
    for file in files {
        let segments: Vec<&str> = file.path.split('/').collect();
        if segments.iter().any(|segment| !is_safe_segment(segment)) {
            return Err(MaterializeError::UnsafeFilePath {
                path: file.path.clone(),
            });
        }
        let from = rel.join(&file.path);
        if segments.len() - 1 > MAX_MATERIALIZE_DEPTH {
            return Err(MaterializeError::TooDeep {
                path: from,
                max: MAX_MATERIALIZE_DEPTH,
            });
        }
        if file.bytes.len() as u64 > MAX_MATERIALIZE_FILE_BYTES {
            return Err(MaterializeError::FileTooLarge {
                path: from,
                max: MAX_MATERIALIZE_FILE_BYTES,
            });
        }
        let (name, dirs) = segments.split_last().unwrap_or((&"", &[]));
        let mut dir = at("reading", rel, out.try_clone())?;
        for segment in dirs {
            let sub = OsStr::new(segment);
            dir = match dir.open_dir_nofollow(sub) {
                Ok(existing) => existing,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    at("creating", &from, create_dir(&dir, sub))?
                }
                Err(error) => return Err(io("reading", &from, error)),
            };
        }
        let mut handle = at("creating", &from, create_file(&dir, OsStr::new(name)))?;
        at("writing", &from, handle.write_all(&file.bytes))?;
        report.files += 1;
    }
    Ok(())
}

fn remove_entry(parent: &Dir, name: &str) -> Result<(), MaterializeError> {
    let rel = Path::new(name);
    match parent.symlink_metadata(name) {
        Ok(metadata) if metadata.is_dir() => at("clearing", rel, parent.remove_dir_all(name)),
        Ok(_) => at("clearing", rel, parent.remove_file_or_symlink(name)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io("reading", rel, error)),
    }
}

fn copy_dir(
    source: &Dir,
    out: &Dir,
    rel: &Path,
    depth: usize,
    report: &mut MaterializeReport,
) -> Result<(), MaterializeError> {
    for entry in at("reading", rel, source.entries())? {
        let entry = at("reading", rel, entry)?;
        let name = entry.file_name();
        let from = rel.join(&name);
        let file_type = at("reading", &from, entry.file_type())?;
        if file_type.is_symlink() {
            report.skipped_symlinks += 1;
        } else if file_type.is_dir() {
            if depth >= MAX_MATERIALIZE_DEPTH {
                return Err(MaterializeError::TooDeep {
                    path: from,
                    max: MAX_MATERIALIZE_DEPTH,
                });
            }
            let child = at("reading", &from, source.open_dir_nofollow(&name))?;
            let child_out = at("creating", &from, create_dir(out, &name))?;
            copy_dir(&child, &child_out, &from, depth + 1, report)?;
        } else if file_type.is_file() && copy_file(source, out, &name, &from)? {
            report.files += 1;
        }
    }
    Ok(())
}

fn copy_file(
    source: &Dir,
    out: &Dir,
    name: &OsString,
    from: &Path,
) -> Result<bool, MaterializeError> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = at("copying", from, source.open_with(name, &options))?;
    let metadata = at("copying", from, file.metadata())?;
    if !metadata.is_file() {
        return Ok(false);
    }
    let too_large = || MaterializeError::FileTooLarge {
        path: from.to_path_buf(),
        max: MAX_MATERIALIZE_FILE_BYTES,
    };
    if metadata.len() > MAX_MATERIALIZE_FILE_BYTES {
        return Err(too_large());
    }
    let mut handle = at("creating", from, create_file(out, name))?;
    let copied = at(
        "copying",
        from,
        std::io::copy(&mut file.take(MAX_MATERIALIZE_FILE_BYTES + 1), &mut handle),
    )?;
    if copied > MAX_MATERIALIZE_FILE_BYTES {
        return Err(too_large());
    }
    Ok(true)
}

fn create_dir(parent: &Dir, name: &OsStr) -> std::io::Result<Dir> {
    parent.create_dir(name)?;
    parent.open_dir_nofollow(name)
}

fn create_file(parent: &Dir, name: &OsStr) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    parent.open_with(name, &options)
}

fn at<T>(
    context: &'static str,
    path: &Path,
    result: std::io::Result<T>,
) -> Result<T, MaterializeError> {
    result.map_err(|source| io(context, path, source))
}

fn io(context: &'static str, path: &Path, source: std::io::Error) -> MaterializeError {
    MaterializeError::Io {
        context,
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
#[path = "materialize_tests.rs"]
mod tests;
