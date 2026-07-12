//! Model resolution and caching. Implements [`drishti_core::ModelSource`] with
//! one present-or-fetch rule (ADR-004): use the file if it is already here,
//! download it first if it is not, verify any configured hash strictly, and
//! fail loudly otherwise. No model identity is hardcoded; everything comes from
//! the [`Artifact`] handed in.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use drishti_core::config::{Artifact, SourceKind};
use drishti_core::error::ModelError;
use drishti_core::ModelSource;
use sha2::{Digest, Sha256};

/// Filesystem-backed source. Resolves local paths directly and caches remote
/// downloads under `cache_dir`.
pub struct FsSource {
    cache_dir: PathBuf,
    client: reqwest::blocking::Client,
}

impl FsSource {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            cache_dir,
            client: reqwest::blocking::Client::new(),
        }
    }

    /// Use the platform cache directory (`$XDG_CACHE_HOME` or `~/.cache`) under
    /// `drishti/models`, unless an explicit directory is given.
    pub fn with_optional_cache(cache_dir: Option<PathBuf>) -> Self {
        Self::new(cache_dir.unwrap_or_else(default_cache_dir))
    }
}

impl ModelSource for FsSource {
    fn fetch(&self, id: &str, artifact: &Artifact) -> Result<PathBuf, ModelError> {
        match artifact.source {
            SourceKind::Local => {
                let path = PathBuf::from(&artifact.location);
                if !path.exists() {
                    return Err(ModelError::NotFound {
                        id: id.to_string(),
                        location: artifact.location.clone(),
                    });
                }
                if let Some(expected) = &artifact.sha256 {
                    verify(&path, expected, id)?;
                }
                Ok(path)
            }
            SourceKind::Remote => self.fetch_remote(id, artifact),
        }
    }
}

impl FsSource {
    fn fetch_remote(&self, id: &str, artifact: &Artifact) -> Result<PathBuf, ModelError> {
        let dir = self.cache_dir.join(sanitize(id));
        let dest = dir.join(filename_for(&artifact.location));

        // Present: a cached file that matches its hash (or has no hash to check)
        // is used directly, no network.
        if dest.exists() {
            match &artifact.sha256 {
                Some(expected) if verify(&dest, expected, id).is_ok() => return Ok(dest),
                Some(_) => { /* stale or corrupt, fall through and re-download */ }
                None => return Ok(dest),
            }
        }

        // Absent: download first, then verify, then atomically place.
        fs::create_dir_all(&dir).map_err(|e| ModelError::Io {
            path: dir.clone(),
            source: e,
        })?;
        let tmp = dest.with_extension("part");

        let mut resp = self
            .client
            .get(&artifact.location)
            .send()
            .and_then(|r| r.error_for_status())
            .map_err(|e| ModelError::DownloadFailed {
                id: id.to_string(),
                location: artifact.location.clone(),
                source: Box::new(e),
            })?;

        {
            let mut file = fs::File::create(&tmp).map_err(|e| ModelError::Io {
                path: tmp.clone(),
                source: e,
            })?;
            resp.copy_to(&mut file)
                .map_err(|e| ModelError::DownloadFailed {
                    id: id.to_string(),
                    location: artifact.location.clone(),
                    source: Box::new(e),
                })?;
        }

        if let Some(expected) = &artifact.sha256 {
            verify(&tmp, expected, id)?;
        }

        fs::rename(&tmp, &dest).map_err(|e| ModelError::Io {
            path: dest.clone(),
            source: e,
        })?;
        Ok(dest)
    }
}

/// Compute the SHA-256 of a file and compare to the expected hex value.
fn verify(path: &Path, expected: &str, id: &str) -> Result<(), ModelError> {
    let actual = sha256_file(path)?;
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(ModelError::IntegrityCheckFailed {
            id: id.to_string(),
            expected: expected.to_string(),
            actual,
        })
    }
}

fn sha256_file(path: &Path) -> Result<String, ModelError> {
    let mut file = fs::File::open(path).map_err(|e| ModelError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf).map_err(|e| ModelError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Last path segment of a URL, sanitised; falls back to a hash of the URL when
/// the path has no usable segment.
fn filename_for(location: &str) -> String {
    let trimmed = location.split(['?', '#']).next().unwrap_or(location);
    let segment = trimmed.rsplit('/').next().unwrap_or("");
    if segment.is_empty() {
        let mut hasher = Sha256::new();
        hasher.update(location.as_bytes());
        format!("{}.bin", &hex::encode(hasher.finalize())[..16])
    } else {
        sanitize(segment)
    }
}

/// Keep only filesystem-safe characters so a model id or filename cannot escape
/// the cache directory.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

fn default_cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(|| PathBuf::from(".cache"));
    base.join("drishti").join("models")
}
