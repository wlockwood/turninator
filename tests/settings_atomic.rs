#![cfg(windows)]
use std::{
    fs::{self, OpenOptions},
    os::windows::fs::OpenOptionsExt,
    time::Duration,
};
use turninator::{export::encode::Scratch, settings::atomic_write};
fn locked(path: &std::path::Path) -> std::fs::File {
    OpenOptions::new()
        .read(true)
        .share_mode(3)
        .open(path)
        .unwrap()
}
#[test]
fn atomic_json_replace_survives_a_brief_external_file_lock() {
    let scratch = Scratch::new(&std::env::temp_dir()).unwrap();
    let path = scratch.0.join("frames.json");
    fs::write(&path, b"old valid manifest").unwrap();
    let lock = locked(&path);
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(60));
        drop(lock);
    });
    let result = atomic_write(&path, b"new complete manifest");
    release.join().unwrap();
    result.unwrap();
    assert_eq!(fs::read(path).unwrap(), b"new complete manifest");
}
#[test]
fn permanent_lock_preserves_previous_manifest_and_cleans_its_temporary() {
    let scratch = Scratch::new(&std::env::temp_dir()).unwrap();
    let path = scratch.0.join("frames.json");
    fs::write(&path, b"old valid manifest").unwrap();
    let _lock = locked(&path);
    let start = std::time::Instant::now();
    assert!(atomic_write(&path, b"replacement").is_err());
    assert!(start.elapsed() < Duration::from_secs(2));
    assert_eq!(fs::read(&path).unwrap(), b"old valid manifest");
    assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 1);
}
