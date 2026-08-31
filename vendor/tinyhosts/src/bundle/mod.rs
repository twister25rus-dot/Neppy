//! The files that make up a deployment.
//!
//! A [`Bundle`] is the source of a Next.js application as the provider will
//! receive it: relative paths, slash separated, with contents in memory. Every
//! provider this crate targets builds from source rather than from a pre-built
//! artifact, so the bundle is what a repository looks like — `package.json`,
//! `next.config.js`, `app/`, `public/` — and not a `.next` directory.
//!
//! [`Bundle::from_dir`] therefore skips what a build produces or a package
//! manager fetches: see [`EXCLUDED`]. Uploading `node_modules` is the mistake
//! this list exists to prevent, and it is not a small one — it is usually
//! several hundred megabytes of files the builder is about to fetch itself.
//!
//! Contents cross a process boundary as base64. A JSON array of byte-integers
//! costs roughly three characters per byte, and a bundle is the largest payload
//! this crate moves.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Directory and file names [`Bundle::from_dir`] never descends into or reads.
///
/// Every entry is either a build output, a dependency cache, or version-control
/// metadata: reproducible from the sources beside it, and never part of what a
/// provider needs to build.
pub const EXCLUDED: &[&str] = &[
    "node_modules",
    ".git",
    ".next",
    ".turbo",
    ".vercel",
    ".netlify",
    ".wrangler",
    ".svelte-kit",
    "out",
    "coverage",
    ".DS_Store",
    ".env",
    ".env.local",
    ".env.development",
    ".env.development.local",
    ".env.production",
    ".env.production.local",
    ".env.test",
    ".env.test.local",
];

/// One file in a deployment.
///
/// The fields are private and validated at construction: a `SiteFile` cannot
/// hold a path that is blank, absolute, or climbs out of the bundle, whether it
/// is built through [`SiteFile::new`] or read back from a deserialized
/// [`Bundle`]. A public `path` field would let a caller — or a JSON payload
/// crossing the RPC boundary — set one directly and skip that check.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawSiteFile", into = "RawSiteFile")]
pub struct SiteFile {
    path: String,
    contents: Vec<u8>,
}

/// The wire shape of [`SiteFile`], validated through [`SiteFile::new`] on the
/// way in.
#[derive(Serialize, Deserialize)]
struct RawSiteFile {
    /// The path the file takes inside the deployment, relative and slash
    /// separated.
    path: String,
    /// The file's bytes, carried as base64 in serialized form.
    #[serde(with = "base64_bytes")]
    contents: Vec<u8>,
}

impl TryFrom<RawSiteFile> for SiteFile {
    type Error = Error;

    fn try_from(raw: RawSiteFile) -> Result<Self> {
        Self::new(raw.path, raw.contents)
    }
}

impl From<SiteFile> for RawSiteFile {
    fn from(file: SiteFile) -> Self {
        Self {
            path: file.path,
            contents: file.contents,
        }
    }
}

impl SiteFile {
    /// A file at `path` holding `contents`.
    ///
    /// Backslashes in `path` become slashes, so a bundle built on Windows
    /// deploys the same tree as one built anywhere else.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidBundlePath`] when the path is blank, absolute, or
    /// climbs out of the bundle with `..`.
    pub fn new(path: impl Into<String>, contents: impl Into<Vec<u8>>) -> Result<Self> {
        let supplied = path.into();
        let normalized = supplied.replace('\\', "/");
        let trimmed = normalized.trim().trim_start_matches("./");

        let rejected = trimmed.is_empty()
            || trimmed.starts_with('/')
            || trimmed.split('/').any(|segment| segment == "..");
        if rejected {
            return Err(Error::InvalidBundlePath { path: supplied });
        }

        Ok(Self {
            path: trimmed.to_owned(),
            contents: contents.into(),
        })
    }

    /// The path the file takes inside the deployment, relative and slash
    /// separated.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The file's bytes.
    #[must_use]
    pub fn contents(&self) -> &[u8] {
        &self.contents
    }

    /// The file's size in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.contents.len()
    }

    /// Whether the file is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.contents.is_empty()
    }
}

/// Prints the path and the size, never the bytes.
///
/// A derived `Debug` on a bundle would render an entire application into a log
/// line, and any `.env`-shaped file inside it.
impl std::fmt::Debug for SiteFile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SiteFile")
            .field("path", &self.path)
            .field("bytes", &self.contents.len())
            .finish()
    }
}

/// The files of one deployment.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<SiteFile>", into = "Vec<SiteFile>")]
pub struct Bundle {
    files: Vec<SiteFile>,
}

impl Bundle {
    /// An empty bundle.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A bundle holding exactly `files`.
    #[must_use]
    pub fn from_files(files: Vec<SiteFile>) -> Self {
        Self { files }
    }

    /// Adds a file, replacing any file already at that path.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidBundlePath`] when the path is not a relative path
    /// inside the bundle.
    pub fn insert(&mut self, path: impl Into<String>, contents: impl Into<Vec<u8>>) -> Result<()> {
        let file = SiteFile::new(path, contents)?;
        match self.files.iter_mut().find(|held| held.path == file.path) {
            Some(held) => *held = file,
            None => self.files.push(file),
        }
        Ok(())
    }

    /// Reads a directory tree into a bundle, skipping [`EXCLUDED`] entries.
    ///
    /// Symbolic links are skipped rather than followed: a link is either
    /// redundant with a file already in the tree or points outside it, and
    /// following one is how a bundle acquires a file the caller never meant to
    /// publish.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ReadBundle`] when the tree cannot be walked or a file
    /// cannot be read, and [`Error::EmptyBundle`] when nothing was collected.
    pub fn from_dir(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        let mut bundle = Self::new();
        collect(root, root, &mut bundle)?;

        if bundle.is_empty() {
            return Err(Error::EmptyBundle);
        }
        Ok(bundle)
    }

    /// The files, in insertion order.
    #[must_use]
    pub fn files(&self) -> &[SiteFile] {
        &self.files
    }

    /// How many files the bundle holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether the bundle holds no files.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// The bundle's total size in bytes.
    #[must_use]
    pub fn total_bytes(&self) -> usize {
        self.files.iter().map(SiteFile::len).sum()
    }
}

impl TryFrom<Vec<SiteFile>> for Bundle {
    type Error = Error;

    /// Collapses any duplicate path to its last occurrence, the same rule
    /// [`Bundle::insert`] applies one file at a time. Each `SiteFile` already
    /// carries a validated path, so this only re-runs [`SiteFile::new`]'s cheap
    /// normalization, not the check itself.
    fn try_from(files: Vec<SiteFile>) -> Result<Self> {
        let mut bundle = Self::new();
        for file in files {
            bundle.insert(file.path, file.contents)?;
        }
        Ok(bundle)
    }
}

impl From<Bundle> for Vec<SiteFile> {
    fn from(bundle: Bundle) -> Self {
        bundle.files
    }
}

/// Walks `directory`, adding every readable file below it to `bundle`.
fn collect(root: &Path, directory: &Path, bundle: &mut Bundle) -> Result<()> {
    let entries = std::fs::read_dir(directory).map_err(|error| read_error(directory, &error))?;

    for entry in entries {
        let entry = entry.map_err(|error| read_error(directory, &error))?;
        let path = entry.path();

        let name = entry.file_name().to_string_lossy().into_owned();
        if EXCLUDED.contains(&name.as_str()) {
            continue;
        }

        let file_type = entry
            .file_type()
            .map_err(|error| read_error(&path, &error))?;

        if file_type.is_symlink() {
            continue;
        }

        if file_type.is_dir() {
            collect(root, &path, bundle)?;
            continue;
        }

        let relative = path
            .strip_prefix(root)
            .map_err(|error| read_error(&path, &error))?;
        let contents = std::fs::read(&path).map_err(|error| read_error(&path, &error))?;

        bundle.insert(relative.to_string_lossy().into_owned(), contents)?;
    }

    Ok(())
}

/// The failure reported when part of a directory tree cannot be read.
fn read_error(path: &Path, reason: &dyn std::fmt::Display) -> Error {
    Error::ReadBundle {
        path: path.display().to_string(),
        reason: reason.to_string(),
    }
}

/// Serializes file contents as base64 rather than as an array of integers.
mod base64_bytes {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<u8>, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        STANDARD
            .decode(encoded.as_bytes())
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod test;
