use knowledge_builder::{build_package, Profile};
use serde_json::{json, Value};
use std::{fs, path::Path, process::Command};

fn write_profile(directory: &Path, yaml: &str) -> std::path::PathBuf {
    let profile = directory.join("profile.yml");
    fs::write(&profile, yaml).unwrap();
    profile
}

fn fixture_catalog() -> Value {
    let mut catalog: Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/catalog-valid.json")).unwrap();
    catalog["sources"].as_array_mut().unwrap().push(json!({
        "id": "jenkins-core-fixture",
        "kind": "jenkins-core",
        "name": "Jenkins Core",
        "version": "2.479.1",
        "revision": "2.479.1"
    }));
    catalog["symbols"].as_array_mut().unwrap().push(json!({
        "id": "jenkins-core-fixture:Run",
        "name": "Run",
        "kind": "class",
        "sourceId": "jenkins-core-fixture",
        "confidence": "verified",
        "provenance": ["Jenkins Core fixture"]
    }));
    catalog
}

fn write_fixture(directory: &Path) {
    fs::write(
        directory.join("fixture.json"),
        serde_json::to_vec(&fixture_catalog()).unwrap(),
    )
    .unwrap();
}

#[test]
fn rejects_unknown_fields_at_every_profile_nesting_level() {
    let profiles = [
        ("profile: strict\nunknown: true\n", "unknown"),
        ("profile: strict\njenkins:\n  typo: true\n", "typo"),
        (
            "profile: strict\njenkins:\n  controllers:\n    - id: ci\n      url: https://jenkins.example.test\n      typo: true\n",
            "typo",
        ),
        (
            "profile: strict\nlibraries:\n  - name: lib\n    url: https://git.example.test/lib.git\n    revision: main\n    typo: true\n",
            "typo",
        ),
        ("profile: strict\nfixtures: [fixture.json]\npackage:\n  typo: true\n", "typo"),
    ];

    for (yaml, field) in profiles {
        let error = serde_yaml::from_str::<Profile>(yaml).unwrap_err();
        assert!(error.to_string().contains(field), "{error}");
    }
}

#[test]
fn rejects_unknown_and_duplicate_package_sections() {
    let unknown = "profile: strict\nfixtures: [fixture.json]\npackage:\n  include: [plugins, made-up]\n";
    assert!(serde_yaml::from_str::<Profile>(unknown).is_err());

    let directory = tempfile::tempdir().unwrap();
    write_fixture(directory.path());
    let empty = write_profile(
        directory.path(),
        "profile: strict\nfixtures: [fixture.json]\npackage:\n  include: []\n",
    );
    let error = build_package(&empty, &directory.path().join("empty-package")).unwrap_err();
    assert!(error.to_string().contains("omit the field"));

    let duplicate = write_profile(
        directory.path(),
        "profile: strict\nfixtures: [fixture.json]\npackage:\n  include: [plugins, plugins]\n",
    );
    let error = build_package(&duplicate, &directory.path().join("duplicate-package")).unwrap_err();
    assert!(error.to_string().contains("duplicate section selector"));
}

#[test]
fn package_include_filters_sources_symbols_and_controller_metadata() {
    let directory = tempfile::tempdir().unwrap();
    write_fixture(directory.path());
    let profile = write_profile(
        directory.path(),
        "profile: selection-test\nfixtures:\n  - fixture.json\npackage:\n  id: selected-package\n  include:\n    - plugins\n    - shared-libraries\n",
    );
    let output = directory.path().join("package");
    build_package(&profile, &output).unwrap();

    let catalog: Value =
        serde_json::from_slice(&fs::read(output.join("catalog.json")).unwrap()).unwrap();
    let source_ids: Vec<_> = catalog["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|source| source["id"].as_str().unwrap())
        .collect();
    assert_eq!(source_ids, vec!["platform-lib", "workflow-basic-steps"]);
    assert!(catalog.get("controller").is_none());
    assert!(catalog["symbols"].as_array().unwrap().iter().all(|symbol| {
        matches!(
            symbol["sourceId"].as_str().unwrap(),
            "platform-lib" | "workflow-basic-steps"
        )
    }));
    assert_eq!(catalog["catalogId"], "selected-package");
}

#[test]
fn controller_selection_keeps_snapshot_but_excludes_unselected_plugin_inventory() {
    let directory = tempfile::tempdir().unwrap();
    write_fixture(directory.path());
    let profile = write_profile(
        directory.path(),
        "profile: controller-only\nfixtures: [fixture.json]\npackage:\n  include: [controller]\n",
    );
    let output = directory.path().join("package");
    build_package(&profile, &output).unwrap();

    let catalog: Value =
        serde_json::from_slice(&fs::read(output.join("catalog.json")).unwrap()).unwrap();
    let source_ids: Vec<_> = catalog["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|source| source["id"].as_str().unwrap())
        .collect();
    assert_eq!(source_ids, vec!["controller"]);
    assert_eq!(catalog["controller"]["identity"]["id"], "jenkins-ci");
    assert_eq!(catalog["controller"]["plugins"], json!([]));
    assert!(catalog["symbols"].as_array().unwrap().iter().all(|symbol| {
        symbol["sourceId"] == "controller"
    }));
}

#[test]
fn unmatched_explicit_selector_is_an_error() {
    let directory = tempfile::tempdir().unwrap();
    let mut catalog = fixture_catalog();
    catalog["sources"] = json!([]);
    catalog["symbols"] = json!([]);
    catalog.as_object_mut().unwrap().remove("controller");
    fs::write(
        directory.path().join("fixture.json"),
        serde_json::to_vec(&catalog).unwrap(),
    )
    .unwrap();
    let profile = write_profile(
        directory.path(),
        "profile: unmatched\nfixtures: [fixture.json]\npackage:\n  include: [plugins]\n",
    );
    let error = build_package(&profile, &directory.path().join("package")).unwrap_err();
    assert!(error.to_string().contains("selector 'plugins' matched no sources"));
}

#[test]
fn rejects_mixed_fixture_and_live_collection_before_network_access() {
    let directory = tempfile::tempdir().unwrap();
    let profile = write_profile(
        directory.path(),
        "profile: mixed\njenkins:\n  controllers:\n    - id: ci\n      url: https://user:secret@jenkins.example.test\nfixtures: [fixture.json]\n",
    );
    let error = build_package(&profile, &directory.path().join("package")).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("cannot be combined"));
    assert!(!message.contains("secret"));
}

#[test]
fn validates_duplicate_fixture_paths_and_package_identity() {
    let directory = tempfile::tempdir().unwrap();
    let duplicate = write_profile(
        directory.path(),
        "profile: duplicate\nfixtures: [fixture.json, fixture.json]\n",
    );
    let error = build_package(&duplicate, &directory.path().join("duplicate-package")).unwrap_err();
    assert!(error.to_string().contains("duplicate fixture path"));

    let empty_id = write_profile(
        directory.path(),
        "profile: invalid-id\nfixtures: [fixture.json]\npackage:\n  id: '  '\n",
    );
    let error = build_package(&empty_id, &directory.path().join("empty-id-package")).unwrap_err();
    assert!(error.to_string().contains("package.id"));
}

#[test]
fn cli_returns_actionable_nonzero_error_for_unknown_profile_field() {
    let directory = tempfile::tempdir().unwrap();
    let profile = write_profile(directory.path(), "profile: cli-test\npackge:\n  id: typo\n");
    let output = Command::new(env!("CARGO_BIN_EXE_knowledge-builder"))
        .arg("build")
        .arg(&profile)
        .arg(directory.path().join("package"))
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown field"), "{stderr}");
    assert!(!directory.path().join("package").exists());
}
