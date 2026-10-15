//! Only explicitly marked immediate child directories belong to this cache.
use crate::{Result, settings::atomic_write};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobRecord {
    pub schema_version: u32,
    pub label: String,
    pub state: String,
    pub destination: PathBuf,
    pub error: Option<String>,
}
pub struct CacheStore {
    root: PathBuf,
}
impl CacheStore {
    pub fn new(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root)?;
        Ok(Self {
            root: fs::canonicalize(root)?,
        })
    }
    pub fn create_job(&self, name: &str) -> Result<PathBuf> {
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err("invalid job directory name".into());
        }
        let path = self.root.join(name);
        fs::create_dir(&path)?;
        atomic_write(&path.join(".turninator-job"), b"Turninator job v1\n")?;
        Ok(path)
    }
    /// Reopen only our own marked directory; never adopt a foreign collision.
    pub fn render_job(&self, key: &str) -> Result<PathBuf> {
        if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("invalid render cache key".into());
        }
        let name = format!("render-{key}");
        let path = self.root.join(&name);
        match fs::symlink_metadata(&path) {
            Ok(_) => self.owned(&path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => self.create_job(&name),
            Err(error) => Err(error.into()),
        }
    }
    fn owned(&self, path: &Path) -> Result<PathBuf> {
        let metadata = fs::symlink_metadata(path)?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("cache cannot own reparse points".into());
            }
        }
        if metadata.file_type().is_symlink() {
            return Err("cache cannot own symbolic links".into());
        }
        let resolved = fs::canonicalize(path)?;
        if resolved.parent() != Some(self.root.as_path())
            || crate::settings::read_bounded(&resolved.join(".turninator-job"), 64)?
                != b"Turninator job v1\n"
        {
            return Err("directory is not an owned immediate cache job".into());
        }
        Ok(resolved)
    }
    pub fn owned_job(&self, path: &Path) -> Result<PathBuf> {
        self.owned(path)
    }
    /// Keep the lock outside the job so storage removal can hold it throughout.
    /// OS locks are released on process exit, including a crash.
    pub fn lock_job(&self, path: &Path) -> Result<fs::File> {
        let path = self.owned(path)?;
        let name = path
            .file_name()
            .ok_or("job name missing")?
            .to_string_lossy();
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join(format!(".{name}.lock")))?;
        file.try_lock().map_err(|_| "This retained render is in use by another export. Wait for it to finish before exporting or removing it.")?;
        Ok(file)
    }
    pub fn save_record(&self, path: &Path, record: &JobRecord) -> Result<()> {
        let path = self.owned(path)?;
        atomic_write(
            &path.join("job-record.json"),
            &serde_json::to_vec_pretty(record)?,
        )
    }
    pub fn records(&self) -> Result<Vec<(PathBuf, JobRecord)>> {
        let mut records = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let path = entry?.path();
            if self.owned(&path).is_err() {
                continue;
            }
            if let Ok(bytes) =
                crate::settings::read_bounded(&path.join("job-record.json"), 1024 * 1024)
                && bytes.len() <= 1024 * 1024
                && let Ok(record) = serde_json::from_slice::<JobRecord>(&bytes)
                && record.schema_version == 1
            {
                records.push((path, record));
            }
        }
        records.sort_by(|a, b| b.0.cmp(&a.0));
        Ok(records)
    }
    pub fn remove_job(&self, path: &Path, active: Option<&Path>) -> Result<()> {
        let path = self.owned(path)?;
        if active.and_then(|p| fs::canonicalize(p).ok()).as_ref() == Some(&path) {
            return Err("cannot remove the active job".into());
        }
        let _guard = self.lock_job(&path)?;
        // Reject any nested link/junction before recursion, including links introduced after import.
        check_tree(&path)?;
        fs::remove_dir_all(path)?;
        Ok(())
    }
    pub fn usage(&self) -> Result<u64> {
        let mut size = 0u64;
        for entry in fs::read_dir(&self.root)? {
            let path = entry?.path();
            if self.owned(&path).is_ok() {
                size = size
                    .checked_add(tree_size(&path)?)
                    .ok_or("cache size overflow")?;
            }
        }
        Ok(size)
    }
    pub fn check_budget(&self, budget: u64, reserve: u64) -> Result<()> {
        if self
            .usage()?
            .checked_add(reserve)
            .is_none_or(|v| v > budget)
        {
            return Err("Job storage budget exceeded. Remove inactive jobs in Tools / storage or increase the budget.".into());
        }
        Ok(())
    }
}
fn check_tree(path: &Path) -> Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("job contains a reparse point".into());
            }
        }
        if metadata.file_type().is_symlink() {
            return Err("job contains a symbolic link".into());
        }
        if metadata.is_dir() {
            check_tree(&entry.path())?;
        }
    }
    Ok(())
}
fn tree_size(path: &Path) -> Result<u64> {
    check_tree(path)?;
    let mut size = 0u64;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        size = size
            .checked_add(if metadata.is_dir() {
                tree_size(&entry.path())?
            } else {
                metadata.len()
            })
            .ok_or("cache size overflow")?;
    }
    Ok(size)
}
