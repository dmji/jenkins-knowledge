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
    catalog["coverage"] = json!([
        {"sourceId": "controller", "surface": "pipeline-gdsl", "status": "inspected"},
        {"sourceId": "platform-lib", "surface": "shared-library", "status": "inspected"},
        {"sourceId": "workflow-basic-steps", "surface": "plugin-inventory", "status": "inspected"},
        {"sourceId": "jenkins-core-fixture", "surface": "core-version", "status": "inspected"}
    ]);
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
    let unknown =
        "profile: strict\nfixtures: [fixture.json]\npackage:\n  include: [plugins, made-up]\n";
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
    let coverage_ids: Vec<_> = catalog["coverage"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["sourceId"].as_str().unwrap())
        .collect();
    assert_eq!(coverage_ids, vec!["platform-lib", "workflow-basic-steps"]);
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
    assert!(catalog["symbols"]
        .as_array()
        .unwrap()
        .iter()
        .all(|symbol| { symbol["sourceId"] == "controller" }));
}

#[test]
fn omitted_include_selects_all_available_sections() {
    let directory = tempfile::tempdir().unwrap();
    write_fixture(directory.path());
    let profile = write_profile(
        directory.path(),
        "profile: all-sections\nfixtures: [fixture.json]\n",
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
    assert_eq!(
        source_ids,
        vec![
            "controller",
            "jenkins-core-fixture",
            "platform-lib",
            "workflow-basic-steps"
        ]
    );
    assert_eq!(
        catalog["controller"]["plugins"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn jenkins_core_selection_is_independent_of_controller_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    write_fixture(directory.path());
    let profile = write_profile(
        directory.path(),
        "profile: core-only\nfixtures: [fixture.json]\npackage:\n  include: [jenkins-core]\n",
    );
    let output = directory.path().join("package");
    build_package(&profile, &output).unwrap();

    let catalog: Value =
        serde_json::from_slice(&fs::read(output.join("catalog.json")).unwrap()).unwrap();
    assert_eq!(catalog["sources"].as_array().unwrap().len(), 1);
    assert_eq!(catalog["sources"][0]["kind"], "jenkins-core");
    assert!(catalog.get("controller").is_none());
    assert_eq!(catalog["symbols"].as_array().unwrap().len(), 1);
    assert_eq!(catalog["symbols"][0]["sourceId"], "jenkins-core-fixture");
    assert_eq!(catalog["coverage"].as_array().unwrap().len(), 1);
    assert_eq!(catalog["coverage"][0]["sourceId"], "jenkins-core-fixture");
}

#[test]
fn rejects_invalid_profile_defaults_and_absolute_fixture_paths() {
    let directory = tempfile::tempdir().unwrap();
    let empty = write_profile(directory.path(), "profile: empty\njenkins: {}\n");
    let error = build_package(&empty, &directory.path().join("empty-package")).unwrap_err();
    assert!(error.to_string().contains("configure at least one"));

    let absolute = write_profile(
        directory.path(),
        "profile: absolute\nfixtures: [/tmp/outside.json]\n",
    );
    let error = build_package(&absolute, &directory.path().join("absolute-package")).unwrap_err();
    assert!(error.to_string().contains("relative path"));
}

#[test]
fn unmatched_explicit_selector_is_an_error() {
    let directory = tempfile::tempdir().unwrap();
    let mut catalog = fixture_catalog();
    let library_source = catalog["sources"][1].clone();
    let library_symbol = catalog["symbols"]
        .as_array()
        .unwrap()
        .iter()
        .find(|symbol| symbol["sourceId"] == "platform-lib")
        .unwrap()
        .clone();
    catalog["sources"] = json!([library_source]);
    catalog["symbols"] = json!([library_symbol]);
    catalog["coverage"] = json!([{
        "sourceId": "platform-lib",
        "surface": "shared-library",
        "status": "inspected"
    }]);
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
    assert!(error
        .to_string()
        .contains("selector 'plugins' matched no sources"));
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
fn rejects_duplicate_controller_ids_and_library_names() {
    let directory = tempfile::tempdir().unwrap();
    let duplicate_controllers = write_profile(
        directory.path(),
        "profile: duplicate-controllers\njenkins:\n  controllers:\n    - id: ci\n      url: https://jenkins.example.test\n    - id: ci\n      url: https://jenkins-two.example.test\n",
    );
    let error = build_package(
        &duplicate_controllers,
        &directory.path().join("duplicate-controllers-package"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("duplicate controller id"));

    let duplicate_libraries = write_profile(
        directory.path(),
        "profile: duplicate-libraries\nlibraries:\n  - name: shared\n    url: https://git.example.test/one.git\n    revision: main\n  - name: shared\n    url: https://git.example.test/two.git\n    revision: main\n",
    );
    let error = build_package(
        &duplicate_libraries,
        &directory.path().join("duplicate-libraries-package"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("duplicate Shared Library name"));
}

#[test]
fn rejects_multiple_live_controllers_before_network_collection() {
    let directory = tempfile::tempdir().unwrap();
    let profile = write_profile(
        directory.path(),
        "profile: multiple\njenkins:\n  controllers:\n    - id: one\n      url: https://one.example.test\n    - id: two\n      url: https://two.example.test\n",
    );
    let error = build_package(&profile, &directory.path().join("multiple-package")).unwrap_err();
    assert!(error.to_string().contains("one controller per package"));
}

#[test]
fn rejects_invalid_and_conflicting_library_commit_pins_before_network() {
    let directory = tempfile::tempdir().unwrap();
    let invalid = write_profile(
        directory.path(),
        "profile: invalid-commit\nlibraries:\n  - name: lib\n    url: https://git.example.test/lib.git\n    revision: main\n    commit: not-a-sha\n",
    );
    let error =
        build_package(&invalid, &directory.path().join("invalid-commit-package")).unwrap_err();
    assert!(error.to_string().contains("libraries[0].commit"));

    let conflict = write_profile(
        directory.path(),
        "profile: conflicting-pin\nlibraries:\n  - name: lib\n    url: https://git.example.test/lib.git\n    revision: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n    commit: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n",
    );
    let error =
        build_package(&conflict, &directory.path().join("conflicting-pin-package")).unwrap_err();
    assert!(error
        .to_string()
        .contains("conflicts with the full-SHA revision"));
}

#[test]
fn validates_selector_requirements_before_live_collection() {
    let directory = tempfile::tempdir().unwrap();
    let profile = write_profile(
        directory.path(),
        "profile: wrong-section\nlibraries:\n  - name: lib\n    url: https://git.example.test/lib.git\n    revision: main\npackage:\n  include: [plugins]\n",
    );
    let error =
        build_package(&profile, &directory.path().join("wrong-section-package")).unwrap_err();
    assert!(error
        .to_string()
        .contains("requires at least one Jenkins controller"));
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
