//! Installed package inventory checks before any optional DLL is loaded.
use crate::Result;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs::File, io::Read, path::Path};

#[derive(Deserialize)]
struct Inventory {
    schema_version: u32,
    package_variant: String,
    backends: BTreeMap<String, Backend>,
    files: Vec<Entry>,
}
#[derive(Deserialize)]
struct Backend {
    status: String,
}
#[derive(Deserialize)]
struct Entry {
    path: String,
    sha256: String,
    size_bytes: u64,
    role: String,
}

pub(crate) fn validate_installation(root: &Path, backend: &str) -> Result<()> {
    validate(root, backend).map_err(|error| format!("{backend} package inventory is unavailable: {error}. Repair the verified renderer package beside the application.").into())
}
fn validate(root: &Path, backend: &str) -> Result<()> {
    let manifest = root.join("native-sdk-manifest.json");
    match manifest.try_exists()? {
        // Standalone source/qualification builds use their backend receipts.
        false => return Ok(()),
        true if manifest.canonicalize()? != manifest => {
            return Err("redirected native inventory".into());
        }
        _ => {}
    }
    let inventory: Inventory =
        serde_json::from_slice(&crate::settings::read_bounded(&manifest, 1024 * 1024)?)?;
    if inventory.schema_version != 1 || inventory.files.len() > 4096 {
        return Err("invalid native inventory schema or file count".into());
    }
    if inventory.package_variant == "core" {
        return Err(format!("{backend} is not bundled in this core package").into());
    }
    if inventory.package_variant != "full"
        || inventory
            .backends
            .get(backend)
            .is_none_or(|b| b.status != "qualified")
    {
        return Err("native backend is not qualified in this package".into());
    }
    let prefix = format!("renderers/{backend}/");
    let mut owned = BTreeMap::new();
    for entry in inventory
        .files
        .iter()
        .filter(|e| e.path.starts_with(&prefix))
    {
        let relative = &entry.path[prefix.len()..];
        if relative.contains('\\')
            || relative.split('/').any(|part| {
                part.is_empty()
                    || matches!(part, "." | "..")
                    || part.ends_with(['.', ' '])
                    || part.chars().any(|c| c < ' ' || "<>:\"|?*".contains(c))
            })
        {
            return Err("unsafe native inventory path".into());
        }
        if !matches!(
            entry.role.as_str(),
            "bridge" | "runtime" | "receipt" | "asset"
        ) || owned.insert(relative.to_ascii_lowercase(), entry).is_some()
        {
            return Err("invalid or duplicate native inventory entry".into());
        }
        let path = root.join(&entry.path);
        if path.canonicalize().ok().as_ref() != Some(&path) || !path.is_file() {
            return Err(format!("missing or redirected owned dependency {}", entry.path).into());
        }
        let mut file = File::open(&path)?;
        if file.metadata()?.len() != entry.size_bytes {
            return Err(format!("native dependency size differs: {}", entry.path).into());
        }
        let mut digest = Sha256::new();
        let mut bytes = [0u8; 65536];
        loop {
            let n = file.read(&mut bytes)?;
            if n == 0 {
                break;
            }
            digest.update(&bytes[..n]);
        }
        if format!("{:x}", digest.finalize()) != entry.sha256 {
            return Err(format!("native dependency hash differs: {}", entry.path).into());
        }
    }
    // MSVC redistributables are package dependencies, not Windows components.
    // Requiring their local entries prevents System32 from hiding a broken install.
    for name in [
        format!("tn_renderer_{backend}.dll"),
        format!("{backend}-package.json"),
        "build-attestation.json".into(),
        "msvcp140.dll".into(),
        "vcruntime140.dll".into(),
        "vcruntime140_1.dll".into(),
        if backend == "filament" {
            "msvcp140_2.dll"
        } else {
            "msvcp140_atomic_wait.dll"
        }
        .into(),
    ] {
        if !owned.contains_key(&name) {
            return Err(format!("missing required dependency entry: {name}").into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (crate::export::encode::Scratch, serde_json::Value) {
        let scratch = crate::export::encode::Scratch::new(&std::env::temp_dir()).unwrap();
        let root = scratch.0.canonicalize().unwrap();
        let mut files = vec![];
        for name in [
            "tn_renderer_filament.dll",
            "filament-package.json",
            "build-attestation.json",
            "msvcp140.dll",
            "msvcp140_2.dll",
            "vcruntime140.dll",
            "vcruntime140_1.dll",
        ] {
            let path = format!("renderers/filament/{name}");
            let destination = root.join(&path);
            std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
            std::fs::write(destination, b"qualified").unwrap();
            files.push(serde_json::json!({"path":path,"size_bytes":9,"sha256":format!("{:x}",Sha256::digest(b"qualified")),"role":if name.ends_with(".json") {"receipt"}else{"runtime"}}));
        }
        (
            scratch,
            serde_json::json!({"schema_version":1,"package_variant":"full","backends":{"filament":{"status":"qualified"}},"files":files}),
        )
    }
    fn save(root: &Path, value: &serde_json::Value) {
        std::fs::write(
            root.join("native-sdk-manifest.json"),
            serde_json::to_vec(value).unwrap(),
        )
        .unwrap();
    }
    #[test]
    fn missing_or_corrupt_local_crt_is_rejected_before_system_loader_fallback() {
        let (scratch, inventory) = fixture();
        let root = scratch.0.canonicalize().unwrap();
        save(&root, &inventory);
        assert!(validate_installation(&root, "filament").is_ok());
        let crt = root.join("renderers/filament/msvcp140_2.dll");
        std::fs::remove_file(&crt).unwrap();
        assert!(
            validate_installation(&root, "filament")
                .unwrap_err()
                .to_string()
                .contains("msvcp140_2.dll")
        );
        std::fs::write(crt, b"corrupted").unwrap();
        assert!(
            validate_installation(&root, "filament")
                .unwrap_err()
                .to_string()
                .contains("hash differs")
        );
    }
    #[test]
    fn core_inventory_preserves_an_unavailable_optional_renderer() {
        let scratch = crate::export::encode::Scratch::new(&std::env::temp_dir()).unwrap();
        let root = scratch.0.canonicalize().unwrap();
        assert!(validate_installation(&root, "luxcore").is_ok());
        save(
            &root,
            &serde_json::json!({"schema_version":1,"package_variant":"core","backends":{},"files":[]}),
        );
        assert!(
            validate_installation(&root, "luxcore")
                .unwrap_err()
                .to_string()
                .contains("not bundled")
        );
    }
    #[test]
    fn installed_inventory_rejects_escape_duplicates_and_missing_crt_entries() {
        let (scratch, inventory) = fixture();
        let root = scratch.0.canonicalize().unwrap();
        for alteration in 0..3 {
            let mut value = inventory.clone();
            match alteration {
                0 => value["files"][0]["path"] = "renderers/filament/../../outside.dll".into(),
                1 => {
                    let entry = value["files"][0].clone();
                    value["files"].as_array_mut().unwrap().push(entry);
                }
                _ => {
                    value["files"].as_array_mut().unwrap().remove(4);
                }
            }
            save(&root, &value);
            assert!(validate_installation(&root, "filament").is_err());
        }
    }
}
