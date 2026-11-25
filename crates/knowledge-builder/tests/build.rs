use knowledge_builder::build_package;
use serde_json::Value;
use std::{fs, path::Path};

#[test]
fn builds_a_valid_offline_package_with_reproducible_catalog() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let profile = root.join("profile.yml");
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let a = first.path().join("package");
    let b = second.path().join("package");
    std::env::set_var("SOURCE_DATE_EPOCH", "0");
    build_package(&profile, &a).unwrap();
    build_package(&profile, &b).unwrap();
    let catalog_a = fs::read(a.join("catalog.json")).unwrap();
    let catalog_b = fs::read(b.join("catalog.json")).unwrap();
    assert_eq!(catalog_a, catalog_b);
    let manifest: Value = serde_json::from_slice(&fs::read(a.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["profile"], "fixture-profile");
    assert_eq!(manifest["sourceRevisions"]["fixture-source"], "0123456789012345678901234567890123456789");
    assert_eq!(manifest["checksums"]["catalog.json"].as_str().unwrap().len(), 64);
}

#[test]
fn does_not_publish_when_output_already_exists() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let output = tempfile::tempdir().unwrap();
    assert!(build_package(&root.join("profile.yml"), output.path()).is_err());
}
