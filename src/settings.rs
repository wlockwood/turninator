use crate::{Result, render::ToolPaths};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static WRITE_ID: AtomicU64 = AtomicU64::new(1);

/// Application allocation ceilings, independent of installed physical memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderMemoryAllowance {
    pub host_gib: u32,
    pub gpu_gib: u32,
}
impl Default for RenderMemoryAllowance {
    fn default() -> Self {
        Self {
            host_gib: 8,
            gpu_gib: 6,
        }
    }
}
impl RenderMemoryAllowance {
    pub fn limits(self) -> Result<crate::render::contract::RenderLimits> {
        if !(1..=64).contains(&self.host_gib) || !(1..=48).contains(&self.gpu_gib) {
            return Err("Render memory maximum must be 1–64 GiB host and 1–48 GiB GPU".into());
        }
        Ok(crate::render::contract::RenderLimits {
            host_bytes: u64::from(self.host_gib) * 1024 * 1024 * 1024,
            gpu_bytes: u64::from(self.gpu_gib) * 1024 * 1024 * 1024,
            ..Default::default()
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub schema_version: u32,
    pub tools: ToolPaths,
    pub recent_files: Vec<PathBuf>,
    pub cache_budget_bytes: u64,
    pub auto_preview: bool,
    pub renderers: crate::render::RendererPreferences,
    pub render_memory: RenderMemoryAllowance,
}
impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            tools: ToolPaths::default(),
            recent_files: Vec::new(),
            cache_budget_bytes: 20 * 1024 * 1024 * 1024,
            auto_preview: false,
            renderers: Default::default(),
            render_memory: Default::default(),
        }
    }
}
impl AppSettings {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        if fs::metadata(path)?.len() > 1024 * 1024 {
            return Err("settings file exceeds budget".into());
        }
        let mut settings: Self = serde_json::from_slice(&read_bounded(path, 1024 * 1024)?)?;
        if settings.schema_version != 1 {
            return Err("unsupported settings version".into());
        }
        settings.renderers.validate()?;
        settings.render_memory.limits()?;
        let recents = std::mem::take(&mut settings.recent_files);
        for path in recents.into_iter().rev() {
            settings.remember(path);
        }
        Ok(settings)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.renderers.validate()?;
        self.render_memory.limits()?;
        let bytes = serde_json::to_vec_pretty(self)?;
        if bytes.len() > 1024 * 1024 {
            return Err("settings exceed 1 MiB reload budget".into());
        }
        atomic_write(path, &bytes)
    }
    pub fn remember(&mut self, path: PathBuf) {
        self.recent_files.retain(|p| p != &path);
        self.recent_files.insert(0, path);
        self.recent_files.truncate(10);
    }
}

pub fn default_data_directory() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".turninator"))
        .join("Turninator")
}
pub fn read_bounded(path: &Path, maximum_bytes: u64) -> Result<Vec<u8>> {
    use std::io::Read;
    let file = fs::File::open(path)?;
    let length = file.metadata()?.len();
    if length > maximum_bytes {
        return Err(format!(
            "{} exceeds its {} byte input budget",
            path.display(),
            maximum_bytes
        )
        .into());
    }
    let mut bytes = Vec::with_capacity(length.min(maximum_bytes) as usize);
    file.take(maximum_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum_bytes {
        return Err("input grew beyond its byte budget during read".into());
    }
    Ok(bytes)
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".turninator-save-{}-{}.partial",
        std::process::id(),
        WRITE_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let operation = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            let source: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
            let target: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let ok = unsafe {
                windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                    source.as_ptr(),
                    target.as_ptr(),
                    windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING
                        | windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH,
                )
            };
            if ok == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        #[cfg(not(windows))]
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    operation
}
