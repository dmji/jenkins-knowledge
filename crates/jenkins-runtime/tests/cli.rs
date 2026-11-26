use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, process::Command};

fn package() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let catalog_bytes = include_bytes!("../../../tests/fixtures/catalog-valid.json");
    let catalog: Value = serde_json::from_slice(catalog_bytes).unwrap();
    fs::write(directory.path().join("catalog.json"), catalog_bytes).unwrap();
    let checksum = Sha256::digest(catalog_bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut revisions = serde_json::Map::new();
    let mut provenance = serde_json::Map::new();
    for source in catalog["sources"].as_array().unwrap() {
        let id = source["id"].as_str().unwrap();
        revisions.insert(id.into(), source["revision"].clone());
        provenance.insert(
            id.into(),
            json!({
                "name": source["name"],
                "kind": source["kind"],
                "revision": source["revision"],
                "url": source.get("url").cloned().unwrap_or(Value::Null)
            }),
        );
    }
    fs::write(
        directory.path().join("manifest.json"),
        json!({
            "packageVersion":"1.0.0",
            "catalogSchemaVersion":"1.0.0",
            "profile":"fixture-profile",
            "createdAt":"2026-01-01T00:00:00Z",
            "catalog":"catalog.json",
            "sourceRevisions":revisions,
            "provenance":provenance,
            "checksums":{"catalog.json":checksum},
            "toolVersions":{}
        })
        .to_string(),
    )
    .unwrap();
    directory
}

fn run(package: &tempfile::TempDir, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_jenkins-runtime"))
        .arg("--package")
        .arg(package.path())
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn cli_exposes_controller_plugin_source_type_and_context_queries() {
    let package = package();
    let filtered = run(
        &package,
        &[
            "list",
            "--controller",
            "jenkins-ci",
            "--source-kind",
            "jenkins-controller",
            "--context",
            "steps",
            "--kind",
            "step",
        ],
    );
    assert!(
        filtered.status.success(),
        "{}",
        String::from_utf8_lossy(&filtered.stderr)
    );
    let response: Value = serde_json::from_slice(&filtered.stdout).unwrap();
    assert_eq!(response["results"].as_array().unwrap().len(), 1);
    assert_eq!(response["results"][0]["name"], "sh");

    let detail = run(&package, &["get", "deployApp"]);
    assert!(
        detail.status.success(),
        "{}",
        String::from_utf8_lossy(&detail.stderr)
    );
    let detail: Value = serde_json::from_slice(&detail.stdout).unwrap();
    assert_eq!(detail["profile"], "fixture-profile");
    assert_eq!(
        detail["source"]["manifestProvenance"]["kind"],
        "shared-library"
    );
    assert_eq!(detail["confidence"], "inferred");

    for (command, field, expected) in [
        ("controller", "controller", "2.479.1"),
        ("plugins", "plugins", "workflow-basic-steps"),
        ("sources", "sources", "platform-lib"),
        ("steps", "steps", "sh"),
        ("globals", "globals", "deployApp"),
        ("declarative", "constructs", "agent"),
        ("types", "types", "ShellStep"),
        ("library-symbols", "symbols", "deployApp"),
    ] {
        let output = run(&package, &[command]);
        assert!(
            output.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        let rendered = value[field].to_string();
        assert!(
            rendered.contains(expected),
            "{command} did not include {expected}: {rendered}"
        );
    }

    let plugin_symbols = run(
        &package,
        &["plugin-symbols", "--plugin", "workflow-basic-steps"],
    );
    assert!(
        plugin_symbols.status.success(),
        "{}",
        String::from_utf8_lossy(&plugin_symbols.stderr)
    );
    let plugin_symbols: Value = serde_json::from_slice(&plugin_symbols.stdout).unwrap();
    assert_eq!(plugin_symbols["total"], 3);
    assert!(plugin_symbols["symbols"]
        .as_array()
        .unwrap()
        .iter()
        .all(|symbol| symbol["source"]["kind"] == "plugin"));
}

#[test]
fn cli_keeps_profile_isolation_explicit() {
    let package = package();
    let output = run(&package, &["--profile", "another-profile", "list"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not match package profile"));
}
