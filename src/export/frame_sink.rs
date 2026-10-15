//! Host-owned durable frame validation. Producer PNGs remain alive for batching.
use super::{
    encode,
    recipe::{hash, write_json},
};
use crate::{Result, render::contract::*};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Serialize, Deserialize)]
pub struct FrameEntry {
    pub key: FrameKey,
    pub sha256: String,
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    schema_version: u32,
    pixel_recipe_key: String,
    entries: Vec<FrameEntry>,
}

/// Validate each ancestor, not only the final canonical target (junction defense).
pub(crate) fn owned_path(root: &Path, path: &Path) -> Result<PathBuf> {
    let root = fs::canonicalize(root)?;
    let resolved = fs::canonicalize(path)?;
    if !resolved.starts_with(&root) {
        return Err("frame path outside owned staging".into());
    }
    let mut current = path;
    loop {
        let meta = fs::symlink_metadata(current)?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err("staging cannot traverse reparse points".into());
            }
        }
        if meta.file_type().is_symlink() {
            return Err("staging cannot traverse symbolic links".into());
        }
        if fs::canonicalize(current)? == root {
            break;
        }
        current = current.parent().ok_or("staging ancestor missing")?;
    }
    Ok(resolved)
}
pub(crate) fn writable_path(root: &Path, path: &Path) -> Result<()> {
    owned_path(root, path.parent().ok_or("staging parent missing")?)?;
    if fs::symlink_metadata(path).is_ok() {
        owned_path(root, path)?;
    }
    Ok(())
}
pub struct FrameSink<'a> {
    plan: FramePlan,
    root: PathBuf,
    directory: PathBuf,
    recipe: String,
    cancel: &'a AtomicBool,
    entries: BTreeMap<FrameKey, FrameEntry>,
    host_bytes: u64,
}
impl<'a> FrameSink<'a> {
    pub fn new(plan: &FramePlan, context: &RenderContext<'a>) -> Result<Self> {
        if context.cancel.load(Ordering::Acquire) {
            return Err("cancelled".into());
        }
        if plan.frames.is_empty() || plan.frames.len() > context.limits.max_frames as usize * 4 {
            return Err("invalid frame count".into());
        }
        let mut expected = BTreeMap::new();
        let mut bytes = 0u64;
        for frame in &plan.frames {
            if expected.insert(frame.key, frame).is_some()
                || frame.width < 16
                || frame.height < 16
                || frame.width > context.limits.max_output_side
                || frame.height > context.limits.max_output_side
            {
                return Err("invalid or duplicate planned frame".into());
            }
            let size = u64::from(frame.width) * u64::from(frame.height) * 4;
            if size > context.limits.host_bytes || size > 64_000_000 * 4 {
                return Err("frame exceeds image budget".into());
            }
            bytes = bytes
                .checked_add(size * 2)
                .ok_or("frame storage overflow")?;
        }
        if bytes > context.limits.scratch_bytes {
            return Err("frames exceed scratch budget".into());
        }
        for (target, source) in &plan.repeat_of {
            let target_frame = expected.get(target).ok_or("unknown repeat target")?;
            let source_frame = expected.get(source).ok_or("unknown repeat source")?;
            if source.panel != target.panel || source.index >= target.index {
                return Err("repeat must reference an earlier frame in the same panel".into());
            }
            let mut a = serde_json::to_value(target_frame)?;
            let mut b = serde_json::to_value(source_frame)?;
            for key in ["key", "time_seconds"] {
                a.as_object_mut().unwrap().remove(key);
                b.as_object_mut().unwrap().remove(key);
            }
            if a != b {
                return Err("repeat pixel inputs differ".into());
            }
        }
        let root = fs::canonicalize(context.scratch_dir)?;
        let directory = root.join("host-frames");
        fs::create_dir_all(&directory)?;
        owned_path(&root, &directory)?;
        let mut sink = Self {
            plan: plan.clone(),
            root,
            directory,
            recipe: context.pixel_recipe_key.into(),
            cancel: context.cancel,
            entries: BTreeMap::new(),
            host_bytes: context.limits.host_bytes,
        };
        if let Ok(bytes) =
            crate::settings::read_bounded(&sink.directory.join("frames.json"), 8 * 1024 * 1024)
            && let Ok(manifest) = serde_json::from_slice::<Manifest>(&bytes)
        {
            if manifest.schema_version != 3 || manifest.pixel_recipe_key != sink.recipe {
                return Err("retained pixel recipe differs".into());
            }
            for entry in manifest.entries {
                if !expected.contains_key(&entry.key) || sink.entries.contains_key(&entry.key) {
                    return Err("retained frame keys differ".into());
                }
                let path = sink.path(entry.key);
                if sink.validate(entry.key, &path).is_ok()
                    && hash(&path).is_ok_and(|hash| hash == entry.sha256)
                {
                    sink.entries.insert(entry.key, entry);
                }
            }
        }
        Ok(sink)
    }
    pub fn path(&self, key: FrameKey) -> PathBuf {
        self.directory
            .join(format!("{:?}-{:04}.png", key.panel, key.index))
    }
    pub fn contains(&self, key: FrameKey) -> bool {
        self.entries.contains_key(&key)
    }
    pub fn complete(&self) -> bool {
        self.entries.len() == self.plan.frames.len()
    }
    fn check_cancel(&self) -> Result<()> {
        if self.cancel.load(Ordering::Acquire) {
            Err("cancelled".into())
        } else {
            Ok(())
        }
    }
    fn validate(&self, key: FrameKey, path: &Path) -> Result<()> {
        self.check_cancel()?;
        let frame = self
            .plan
            .frames
            .iter()
            .find(|f| f.key == key)
            .ok_or("unknown frame key")?;
        owned_path(&self.root, path)?;
        if fs::metadata(path)?.len() > u64::from(frame.width) * u64::from(frame.height) * 5 + 65536
        {
            return Err("PNG exceeds planned disk byte budget".into());
        }
        let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
        if reader.format() != Some(image::ImageFormat::Png) {
            return Err("frame is not PNG".into());
        }
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(frame.width);
        limits.max_image_height = Some(frame.height);
        limits.max_alloc = Some(self.host_bytes);
        reader.limits(limits);
        let image = reader.decode()?.to_rgba8();
        if image.width() != frame.width || image.height() != frame.height {
            return Err("wrong frame dimensions".into());
        }
        if !self.plan.transparent && image.pixels().any(|p| p[3] != 255) {
            return Err("unexpected alpha in opaque frame".into());
        }
        self.check_cancel()
    }
    fn save(&self) -> Result<()> {
        write_json(
            &self.directory.join("frames.json"),
            &serde_json::to_value(Manifest {
                schema_version: 3,
                pixel_recipe_key: self.recipe.clone(),
                entries: self.entries.values().cloned().collect(),
            })?,
        )
    }
    pub fn accept(&mut self, artifact: FrameArtifact) -> Result<PathBuf> {
        self.check_cancel()?;
        let key = match &artifact {
            FrameArtifact::Image { key, .. } | FrameArtifact::Png { key, .. } => *key,
            FrameArtifact::Metadata { .. } => {
                return Err("metadata is not a completed frame".into());
            }
        };
        let frame = self
            .plan
            .frames
            .iter()
            .find(|f| f.key == key)
            .ok_or("unknown frame key")?;
        if self.entries.contains_key(&key) || self.plan.repeat_of.contains_key(&key) {
            return Err("duplicate/conflicting frame key".into());
        }
        let output = self.path(key);
        let partial = output.with_extension("partial.png");
        writable_path(&self.root, &partial)?;
        writable_path(&self.root, &output)?;
        match artifact {
            FrameArtifact::Metadata { .. } => {
                unreachable!("metadata rejected before pixel staging")
            }
            FrameArtifact::Image { image, .. } => {
                if image.width != frame.width
                    || image.height != frame.height
                    || image.bytes.len() as u64
                        != u64::from(frame.width) * u64::from(frame.height) * 4
                {
                    return Err("wrong frame dimensions or byte count".into());
                }
                image::RgbaImage::from_raw(image.width, image.height, image.bytes)
                    .ok_or("invalid image")?
                    .save(&partial)?;
            }
            FrameArtifact::Png { path, .. } => {
                self.validate(key, &path)?;
                fs::copy(path, &partial)?;
            }
        }
        self.validate(key, &partial)?;
        self.check_cancel()?;
        encode::publish(&partial, &output)?;
        self.entries.insert(
            key,
            FrameEntry {
                key,
                sha256: hash(&output)?,
            },
        );
        self.save()?;
        Ok(output)
    }
    pub fn finish(&mut self) -> Result<()> {
        self.check_cancel()?;
        for (target, source) in self.plan.repeat_of.clone() {
            if self
                .entries
                .get(&target)
                .zip(self.entries.get(&source))
                .is_some_and(|(a, b)| a.sha256 == b.sha256)
            {
                continue;
            }
            if !self.entries.contains_key(&source) {
                return Err("repeat source missing".into());
            }
            let from = self.path(source);
            self.validate(source, &from)?;
            if hash(&from)? != self.entries[&source].sha256 {
                return Err("repeat source pixels changed".into());
            }
            let to = self.path(target);
            let partial = to.with_extension("partial.png");
            writable_path(&self.root, &partial)?;
            writable_path(&self.root, &to)?;
            fs::copy(&from, &partial)?;
            self.validate(target, &partial)?;
            self.check_cancel()?;
            encode::publish(&partial, &to)?;
            self.entries.insert(
                target,
                FrameEntry {
                    key: target,
                    sha256: hash(&to)?,
                },
            );
            self.save()?;
        }
        if !self.complete() {
            return Err("missing planned frame keys".into());
        }
        for entry in self.entries.values() {
            let path = self.path(entry.key);
            self.validate(entry.key, &path)?;
            if hash(&path)? != entry.sha256 {
                return Err("retained frame hash differs".into());
            }
        }
        Ok(())
    }
}
