use catalog_validator::validate_catalog;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, env, fs, path::Path};

mod source_security;

#[derive(Debug, Deserialize)]
pub struct Profile {
    pub profile: String,
    #[serde(default)]
    pub jenkins: JenkinsConfig,
    #[serde(default)]
    pub libraries: Vec<LibraryConfig>,
    #[serde(default)]
    pub fixtures: Vec<String>,
    #[serde(default)]
    pub package: PackageConfig,
}
#[derive(Debug, Default, Deserialize)]
pub struct JenkinsConfig { #[serde(default)] pub controllers: Vec<ControllerConfig> }
#[derive(Debug, Deserialize)]
pub struct ControllerConfig { pub id: String, pub url: String }
#[derive(Debug, Deserialize)]
pub struct LibraryConfig { pub name: String, pub url: String, pub revision: String, pub commit: Option<String> }
#[derive(Debug, Default, Deserialize)]
pub struct PackageConfig { pub id: Option<String>, #[serde(default)] pub include: Vec<String> }

#[derive(Debug)]
pub enum BuildError { Io(String), Profile(String), Source(String), InvalidCatalog(String), OutputExists(String) }
impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(s) => write!(f, "{s}"),
            Self::Profile(s) => write!(f, "profile: {s}"),
            Self::Source(s) => write!(f, "source: {s}"),
            Self::InvalidCatalog(s) => write!(f, "catalog validation failed: {s}"),
            Self::OutputExists(s) => write!(f, "output already exists: {s}"),
        }
    }
}
impl std::error::Error for BuildError {}

pub fn build_package(profile_path: &Path, output: &Path) -> Result<(), BuildError> {
    if output.exists() { return Err(BuildError::OutputExists(output.display().to_string())); }
    let input = fs::read_to_string(profile_path).map_err(|e| BuildError::Io(format!("cannot read {}: {e}", profile_path.display())))?;
    let profile: Profile = serde_yaml::from_str(&input).map_err(|e| BuildError::Profile(e.to_string()))?;
    if profile.profile.trim().is_empty() { return Err(BuildError::Profile("profile identity must not be empty".into())); }
    for controller in &profile.jenkins.controllers {
        source_security::validate_controller_url(&controller.url).map_err(|error| {
            BuildError::Profile(format!("controller '{}' URL: {error}", controller.id))
        })?;
    }
    for library in &profile.libraries {
        source_security::validate_git_url(&library.url).map_err(|error| {
            BuildError::Profile(format!("Shared Library '{}' URL: {error}", library.name))
        })?;
        source_security::validate_git_reference(&library.revision).map_err(|error| {
            BuildError::Profile(format!("Shared Library '{}' revision: {error}", library.name))
        })?;
        if let Some(commit) = &library.commit {
            if commit.len() != 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(BuildError::Profile(format!(
                    "Shared Library '{}' commit must be a full 40-character SHA",
                    library.name
                )));
            }
        }
    }
    let base = profile_path.parent().unwrap_or(Path::new("."));
    let mut catalogs = Vec::new();

    if !profile.fixtures.is_empty() {
        for fixture in &profile.fixtures {
            let path = base.join(fixture);
            let text = fs::read_to_string(&path).map_err(|e| BuildError::Source(format!("fixture {}: {e}", path.display())))?;
            let catalog: Value = serde_json::from_str(&text).map_err(|e| BuildError::Source(format!("fixture {} is invalid JSON: {e}", path.display())))?;
            catalogs.push((fixture.clone(), catalog));
        }
    } else {
        for controller in &profile.jenkins.controllers {
            let catalog = jenkins_gdsl_collector::collect(&controller.url, env::var("JENKINS_USERNAME").ok().as_deref(), env::var("JENKINS_API_TOKEN").ok().as_deref())
                .map_err(|e| BuildError::Source(format!("Jenkins controller '{}': {e}", controller.id)))?;
            catalogs.push((format!("Jenkins controller '{}'", controller.id), catalog));
        }
        for library in &profile.libraries {
            let config = shared_library_scanner::LibraryConfig { name: library.name.clone(), repository: library.url.clone(), reference: library.revision.clone() };
            let result = if let Some(commit) = &library.commit {
                let checkout = checkout_pinned_library(&library.url, commit, &library.name)?;
                shared_library_scanner::scan_checkout(&config, checkout.0.path(), commit)
                    .map_err(|e| BuildError::Source(format!("Shared Library '{}': {e}", library.name)))?
            } else {
                shared_library_scanner::scan_library(&config)
                    .map_err(|e| BuildError::Source(format!("Shared Library '{}': {e}", library.name)))?
            };
            catalogs.push((format!("Shared Library '{}'", library.name), result.catalog));
        }
        for controller in &profile.jenkins.controllers {
            let source_id = format!("declarative-model-{}", controller.id);
            let fragment = declarative_model::catalog_fragment(&source_id);
            let mut model_symbols = fragment["symbols"].as_array().cloned().unwrap_or_default();
            for symbol in &mut model_symbols {
                let local_id = symbol["id"].as_str().unwrap_or("unknown").to_owned();
                symbol["id"] = json!(format!("{source_id}:{local_id}"));
            }
            let catalog = json!({
                "schemaVersion":"1.0.0","catalogId":source_id,"generatedAt":build_timestamp(),
                "sources":[{"id":source_id,"kind":"jenkins-controller","name":format!("Declarative Pipeline model ({})", controller.id),"revision":"model-v1"}],
                "symbols":model_symbols
            });
            catalogs.push((format!("Declarative model for '{}'", controller.id), catalog));
        }
    }
    if catalogs.is_empty() { return Err(BuildError::Profile("configure at least one controller, library, or fixture catalog".into())); }

    let mut sources = Vec::new();
    let mut symbols = Vec::new();
    let mut coverage = Vec::new();
    let mut source_revisions = BTreeMap::new();
    let mut source_origins = BTreeMap::new();
    let mut provenance = BTreeMap::new();
    let mut controller_snapshot: Option<Value> = None;
    let mut controller_metadata: Option<Value> = None;
    for (label, catalog) in catalogs {
        let schema: Value = serde_json::from_str(include_str!("../../../schemas/catalog.schema.json")).expect("bundled catalog schema");
        validate_catalog(&catalog, &schema).map_err(|e| BuildError::Source(format!("{label}: {e}")))?;
        if let Some(controller) = catalog.get("controller") {
            let mut safe_controller = controller.clone();
            if let Some(base_url) = safe_controller["identity"]["baseUrl"].as_str() {
                safe_controller["identity"]["baseUrl"] = json!(source_security::redact_url(base_url));
            }
            if let Some(previous) = &controller_snapshot {
                if previous != &safe_controller {
                    return Err(BuildError::Source(format!(
                        "{label}: controller snapshot differs from the other configured source snapshots; refusing to mix controller identities or revisions"
                    )));
                }
            } else {
                controller_snapshot = Some(safe_controller.clone());
                controller_metadata = Some(safe_controller);
            }
        }
        for source in catalog["sources"].as_array().into_iter().flatten() {
            let mut safe_source = source.clone();
            if let Some(url) = source["url"].as_str() {
                safe_source["url"] = json!(source_security::redact_url(url));
            }
            let id = safe_source["id"].as_str().ok_or_else(|| BuildError::Source(format!("{label}: source has no id")))?;
            let revision = safe_source["revision"].as_str().unwrap_or("unknown");
            if let Some(previous) = source_origins.insert(id.to_owned(), (label.clone(), revision.to_owned())) {
                return Err(BuildError::Source(format!(
                    "source id '{id}' is declared by both '{}' (revision '{}') and '{}' (revision '{}'); source identities and revisions must not be overwritten during merge",
                    previous.0, previous.1, label, revision
                )));
            }
            source_revisions.insert(id.to_owned(), revision.to_owned());
            provenance.insert(id.to_owned(), json!({
                "name":safe_source["name"],
                "kind":safe_source["kind"],
                "revision":safe_source["revision"],
                "url":safe_source.get("url")
            }));
            sources.push(safe_source);
        }
        symbols.extend(catalog["symbols"].as_array().into_iter().flatten().cloned());
        coverage.extend(catalog["coverage"].as_array().into_iter().flatten().cloned());
    }
    sources.sort_by_key(|v| v["id"].as_str().unwrap_or("").to_owned());
    symbols.sort_by_key(|v| v["id"].as_str().unwrap_or("").to_owned());
    coverage.sort_by_key(|v| {
        (
            v["sourceId"].as_str().unwrap_or("").to_owned(),
            v["surface"].as_str().unwrap_or("").to_owned(),
        )
    });
    let mut catalog = json!({
        "schemaVersion":"1.0.0",
        "catalogId":profile.package.id.unwrap_or_else(|| profile.profile.clone()),
        "generatedAt":build_timestamp(),
        "sources":sources,
        "symbols":symbols,
        "coverage":coverage
    });
    if let Some(controller) = controller_metadata {
        catalog["controller"] = controller;
    }
    let schema: Value = serde_json::from_str(include_str!("../../../schemas/catalog.schema.json")).expect("bundled catalog schema");
    validate_catalog(&catalog, &schema).map_err(|e| BuildError::InvalidCatalog(e.to_string()))?;

    let parent = output.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|e| BuildError::Io(e.to_string()))?;
    let staging = tempfile::Builder::new()
        .prefix(".jenkins-knowledge-staging-")
        .tempdir_in(parent)
        .map_err(|e| BuildError::Io(format!("cannot create package staging directory: {e}")))?;
    let staging_path = staging.path();
    let catalog_bytes = format!("{}\n", serde_json::to_string_pretty(&catalog).map_err(|e| BuildError::Io(e.to_string()))?);
    fs::write(staging_path.join("catalog.json"), &catalog_bytes).map_err(|e| BuildError::Io(e.to_string()))?;
    let manifest = json!({
        "packageVersion":"1.0.0",
        "catalogSchemaVersion":"1.0.0",
        "profile":profile.profile,
        "createdAt":build_timestamp(),
        "catalog":"catalog.json",
        "sourceRevisions":source_revisions,
        "provenance":provenance,
        "checksums":{"catalog.json":hex(&Sha256::digest(catalog_bytes.as_bytes()))},
        "toolVersions":{"knowledge-builder":env!("CARGO_PKG_VERSION")}
    });
    let manifest_schema: Value = serde_json::from_str(include_str!("../../../schemas/manifest.schema.json"))
        .expect("bundled manifest schema");
    let manifest_validator = jsonschema::validator_for(&manifest_schema)
        .map_err(|e| BuildError::InvalidCatalog(format!("invalid manifest schema: {e}")))?;
    if let Some(error) = manifest_validator.iter_errors(&manifest).next() {
        return Err(BuildError::InvalidCatalog(format!("manifest: {error}")));
    }
    fs::write(staging_path.join("manifest.json"), format!("{}\n", serde_json::to_string_pretty(&manifest).map_err(|e| BuildError::Io(e.to_string()))?))
        .map_err(|e| BuildError::Io(e.to_string()))?;
    if output.exists() {
        return Err(BuildError::OutputExists(output.display().to_string()));
    }
    fs::rename(staging.path(), output)
        .map_err(|error| BuildError::Io(format!("cannot publish package {}: {error}", output.display())))?;
    Ok(())
}

fn build_timestamp() -> String {
    if let Ok(epoch) = env::var("SOURCE_DATE_EPOCH") {
        if let Ok(seconds) = epoch.parse::<i64>() {
            if let Some(timestamp) = chrono::DateTime::<chrono::Utc>::from_timestamp(seconds, 0) {
                return timestamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            }
        }
    }
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
fn hex(bytes: &[u8]) -> String { bytes.iter().map(|b| format!("{b:02x}")).collect() }

// A pinned commit is checked out into a temporary directory before scan_checkout is called.
#[derive(Debug)]
struct Checkout(tempfile::TempDir);
fn checkout_pinned_library(url: &str, commit: &str, name: &str) -> Result<Checkout, BuildError> {
    let dir = tempfile::Builder::new()
        .prefix("jenkins-knowledge-checkout-")
        .tempdir()
        .map_err(|e| BuildError::Io(format!("cannot create checkout directory: {e}")))?;
    let run = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| BuildError::Source(format!("Shared Library '{name}': cannot run git: {e}")))?;
        if output.success() { Ok(()) } else { Err(BuildError::Source(format!("Shared Library '{name}': git operation failed; remote details are redacted"))) }
    };
    run(&["init","--quiet"])?;
    run(&["remote","add","origin",url])?;
    run(&["fetch","--quiet","--depth","1","origin",commit])?;
    run(&["checkout","--quiet","--detach","FETCH_HEAD"])?;
    Ok(Checkout(dir))
}

#[cfg(test)]
mod tests {
    use super::{build_package, checkout_pinned_library};
    use serde_json::{json, Value};
    use std::fs;

    #[test]
    fn preserves_collection_coverage_in_merged_catalog() {
        let directory = tempfile::tempdir().unwrap();
        let mut catalog: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/catalog-valid.json"
        ))
        .unwrap();
        catalog["coverage"] = json!([{
            "sourceId":"controller",
            "surface":"pipeline-steps",
            "status":"inspected"
        }]);
        fs::write(
            directory.path().join("fixture.json"),
            serde_json::to_vec(&catalog).unwrap(),
        )
        .unwrap();
        fs::write(
            directory.path().join("profile.yml"),
            "profile: coverage-test\nfixtures:\n  - fixture.json\n",
        )
        .unwrap();

        let output = directory.path().join("package");
        build_package(&directory.path().join("profile.yml"), &output).unwrap();
        let merged: Value =
            serde_json::from_slice(&fs::read(output.join("catalog.json")).unwrap()).unwrap();
        assert_eq!(merged["coverage"][0]["surface"], "pipeline-steps");
        assert_eq!(merged["coverage"][0]["status"], "inspected");
        assert_eq!(merged["controller"]["identity"]["id"], "jenkins-ci");
    }

    #[test]
    fn rejects_conflicting_source_revisions_in_fixture_merge() {
        let directory = tempfile::tempdir().unwrap();
        let fixture = include_str!("../../../tests/fixtures/catalog-valid.json");
        let first: Value = serde_json::from_str(fixture).unwrap();
        let mut second = first.clone();
        second["catalogId"] = json!("second-fixture");
        second["sources"][0]["revision"] = json!("2.480.0");

        fs::write(
            directory.path().join("first.json"),
            serde_json::to_vec(&first).unwrap(),
        )
        .unwrap();
        fs::write(
            directory.path().join("second.json"),
            serde_json::to_vec(&second).unwrap(),
        )
        .unwrap();
        fs::write(
            directory.path().join("profile.yml"),
            "profile: conflict-test\nfixtures:\n  - first.json\n  - second.json\n",
        )
        .unwrap();

        let error = build_package(
            &directory.path().join("profile.yml"),
            &directory.path().join("package"),
        )
        .unwrap_err();

        let message = error.to_string();
        assert!(
            message.contains("source id 'controller' is declared by both"),
            "{message}"
        );
        assert!(message.contains("2.479.1"), "{message}");
        assert!(message.contains("2.480.0"), "{message}");
        assert!(!directory.path().join("package").exists());
    }

    #[test]
    fn rejects_credential_bearing_controller_urls_before_collection() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("profile.yml"),
            r#"profile: unsafe
jenkins:
  controllers:
    - id: ci
      url: https://user:secret@jenkins.example.test/
fixtures:
  - unused.json
"#,
        )
        .unwrap();
        let error = build_package(
            &directory.path().join("profile.yml"),
            &directory.path().join("package"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("credentials must be supplied through the runtime environment"));
        assert!(!directory.path().join("package").exists());
    }

    #[test]
    fn strips_request_modifiers_from_catalog_and_manifest_urls() {
        let directory = tempfile::tempdir().unwrap();
        let mut catalog: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/catalog-valid.json"
        ))
        .unwrap();
        catalog["sources"][1]["url"] = json!("https://github.example.test/team/library.git?ref=release#readme");
        fs::write(
            directory.path().join("fixture.json"),
            serde_json::to_vec(&catalog).unwrap(),
        )
        .unwrap();
        fs::write(
            directory.path().join("profile.yml"),
            r#"profile: redaction-test
fixtures:
  - fixture.json
"#,
        )
        .unwrap();

        let output = directory.path().join("package");
        build_package(&directory.path().join("profile.yml"), &output).unwrap();
        let catalog = fs::read_to_string(output.join("catalog.json")).unwrap();
        let manifest = fs::read_to_string(output.join("manifest.json")).unwrap();
        let package = format!("{catalog}{manifest}");
        assert!(!package.contains("?ref=release"));
        assert!(package.contains("https://github.example.test/team/library.git"));
    }

    #[test]
    fn rejects_malformed_fixture_json_without_publishing_a_package() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("broken.json"), "{ not json").unwrap();
        fs::write(
            directory.path().join("profile.yml"),
            r#"profile: malformed
fixtures:
  - broken.json
"#,
        )
        .unwrap();
        let output = directory.path().join("package");
        assert!(build_package(&directory.path().join("profile.yml"), &output).is_err());
        assert!(!output.exists());
    }

    #[test]
    fn rejects_mixed_controller_snapshots_before_merging_sources() {
        let directory = tempfile::tempdir().unwrap();
        let first: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/catalog-valid.json"
        ))
        .unwrap();
        let mut second = first.clone();
        second["catalogId"] = json!("second-snapshot");
        second["controller"]["identity"]["id"] = json!("different-controller");
        fs::write(directory.path().join("first.json"), serde_json::to_vec(&first).unwrap()).unwrap();
        fs::write(directory.path().join("second.json"), serde_json::to_vec(&second).unwrap()).unwrap();
        fs::write(
            directory.path().join("profile.yml"),
            r#"profile: snapshot-conflict
fixtures:
  - first.json
  - second.json
"#,
        )
        .unwrap();

        let error = build_package(
            &directory.path().join("profile.yml"),
            &directory.path().join("package"),
        )
        .unwrap_err();
        assert!(error.to_string().contains("controller snapshot differs"));
        assert!(!directory.path().join("package").exists());
    }

    #[test]
    fn failed_pinned_checkout_removes_its_temporary_directory() {
        use std::collections::BTreeSet;

        let checkout_dirs = || -> BTreeSet<_> {
            fs::read_dir(std::env::temp_dir())
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.file_name().is_some_and(|name| name.to_string_lossy().starts_with("jenkins-knowledge-checkout-")))
                .collect()
        };

        let before = checkout_dirs();
        let error = checkout_pinned_library(
            "ssh://git@127.0.0.1:1/nonexistent/repository.git",
            "0123456789abcdef0123456789abcdef01234567",
            "test-library",
        )
        .unwrap_err();
        assert!(error.to_string().contains("git operation failed"));
        assert_eq!(checkout_dirs(), before);
    }

}
