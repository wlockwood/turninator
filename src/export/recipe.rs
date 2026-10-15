//! Versioned host recipes never relabel older artifacts with a new runtime.
use crate::Result;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::Path};
pub(crate) fn hash(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        let n = file.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        digest.update(&bytes[..n]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
pub(crate) fn write_json(path: &Path, value: &Value) -> Result<()> {
    crate::settings::atomic_write(path, &serde_json::to_vec_pretty(value)?)
}
pub(crate) fn read_json(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&crate::settings::read_bounded(
        path,
        64 * 1024 * 1024,
    )?)?)
}
