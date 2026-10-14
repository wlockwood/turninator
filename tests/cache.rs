use std::{fs, path::PathBuf};
use turninator::cache::{CacheStore, JobRecord};
fn root() -> PathBuf {
    let path = std::env::temp_dir().join(format!("turninator-cache-{}", std::process::id()));
    fs::create_dir_all(&path).unwrap();
    path
}
#[test]
fn cache_only_removes_owned_inactive_jobs_and_rejects_escape() {
    let root = root();
    let store = CacheStore::new(root.join("jobs")).unwrap();
    let job = store.create_job("safe").unwrap();
    fs::write(job.join("frame.png"), b"1234").unwrap();
    let outside = root.join("unrelated");
    fs::create_dir_all(&outside).unwrap();
    assert!(store.remove_job(&outside, None).is_err());
    assert!(store.remove_job(&job, Some(&job)).is_err());
    store.remove_job(&job, None).unwrap();
    assert!(outside.exists());
    assert!(store.create_job("../escape").is_err());
}
#[test]
fn job_ledger_recovers_failure_and_budget_accounts_retained_frames() {
    let root = root();
    let store = CacheStore::new(root.join("ledger-jobs")).unwrap();
    let job = store.create_job("retained").unwrap();
    fs::write(job.join("frame.png"), [0u8; 128]).unwrap();
    store
        .save_record(
            &job,
            &JobRecord {
                schema_version: 1,
                label: "Retryable".into(),
                state: "failed".into(),
                destination: root.join("output.gif"),
                error: Some("encoder missing".into()),
            },
        )
        .unwrap();
    let records = store.records().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].1.state, "failed");
    assert!(store.check_budget(100, 1).is_err());
    assert!(store.check_budget(1_000_000, 1).is_ok());
    store.remove_job(&job, None).unwrap();
}
