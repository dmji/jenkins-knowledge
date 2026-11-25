use catalog_validator::validate_catalog;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, env, fs, path::{Path, PathBuf}};

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
                shared_library_scanner::scan_checkout(&config, &checkout.0, commit)
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
    for (label, catalog) in catalogs {
        let schema: Value = serde_json::from_str(include_str!("../../../schemas/catalog.schema.json")).expect("bundled catalog schema");
        validate_catalog(&catalog, &schema).map_err(|e| BuildError::Source(format!("{label}: {e}")))?;
        for source in catalog["sources"].as_array().into_iter().flatten() {
            let id = source["id"].as_str().ok_or_else(|| BuildError::Source(format!("{label}: source has no id")))?;
            let revision = source["revision"].as_str().unwrap_or("unknown");
            if let Some(previous) = source_origins.insert(id.to_owned(), (label.clone(), revision.to_owned())) {
                return Err(BuildError::Source(format!(
                    "source id '{id}' is declared by both '{}' (revision '{}') and '{}' (revision '{}'); source identities and revisions must not be overwritten during merge",
                    previous.0, previous.1, label, revision
                )));
            }
            source_revisions.insert(id.to_owned(), revision.to_owned());
            provenance.insert(id.to_owned(), json!({"name":source["name"],"kind":source["kind"],"revision":source["revision"],"url":source.get("url")}));
            sources.push(source.clone());
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
    let catalog = json!({
        "schemaVersion":"1.0.0",
        "catalogId":profile.package.id.unwrap_or_else(|| profile.profile.clone()),
        "generatedAt":build_timestamp(),
        "sources":sources,
        "symbols":symbols,
        "coverage":coverage
    });
    let schema: Value = serde_json::from_str(include_str!("../../../schemas/catalog.schema.json")).expect("bundled catalog schema");
    validate_catalog(&catalog, &schema).map_err(|e| BuildError::InvalidCatalog(e.to_string()))?;

    let parent = output.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|e| BuildError::Io(e.to_string()))?;
    let staging = staging_path(output);
    if staging.exists() { fs::remove_dir_all(&staging).map_err(|e| BuildError::Io(e.to_string()))?; }
    fs::create_dir(&staging).map_err(|e| BuildError::Io(e.to_string()))?;
    let catalog_bytes = format!("{}\n", serde_json::to_string_pretty(&catalog).map_err(|e| BuildError::Io(e.to_string()))?);
    fs::write(staging.join("catalog.json"), &catalog_bytes).map_err(|e| BuildError::Io(e.to_string()))?;
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
        let _ = fs::remove_dir_all(&staging);
        return Err(BuildError::InvalidCatalog(format!("manifest: {error}")));
    }
    fs::write(staging.join("manifest.json"), format!("{}\n", serde_json::to_string_pretty(&manifest).map_err(|e| BuildError::Io(e.to_string()))?))
        .map_err(|e| BuildError::Io(e.to_string()))?;
    fs::rename(&staging, output).map_err(|e| { let _ = fs::remove_dir_all(&staging); BuildError::Io(format!("cannot publish package {}: {e}", output.display())) })?;
    Ok(())
}

fn staging_path(output: &Path) -> PathBuf {
    let name = output.file_name().unwrap_or_default().to_string_lossy();
    output.with_file_name(format!(".{name}.staging-{}", std::process::id()))
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
struct Checkout(PathBuf);
impl std::ops::Deref for Checkout { type Target = PathBuf; fn deref(&self) -> &Self::Target { &self.0 } }
impl Drop for Checkout { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
fn checkout_pinned_library(url: &str, commit: &str, name: &str) -> Result<Checkout, BuildError> {
    if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(BuildError::Source(format!("Shared Library '{name}': commit must be a full 40-character SHA")));
    }
    let dir = env::temp_dir().join(format!("jenkins-knowledge-{}-{}", std::process::id(), &commit[..12]));
    if dir.exists() { fs::remove_dir_all(&dir).map_err(|e| BuildError::Io(e.to_string()))?; }
    fs::create_dir_all(&dir).map_err(|e| BuildError::Io(e.to_string()))?;
    let run = |args: &[&str]| {
        let status = std::process::Command::new("git").args(args).current_dir(&dir).status()
            .map_err(|e| BuildError::Source(format!("Shared Library '{name}': cannot run git: {e}")))?;
        if status.success() { Ok(()) } else { Err(BuildError::Source(format!("Shared Library '{name}': git checkout failed"))) }
    };
    run(&["init","--quiet"])?; run(&["remote","add","origin",url])?; run(&["fetch","--quiet","--depth","1","origin",commit])?; run(&["checkout","--quiet","--detach","FETCH_HEAD"])?;
    Ok(Checkout(dir))
}

#[cfg(test)]
mod tests {
    use super::build_package;
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
}
