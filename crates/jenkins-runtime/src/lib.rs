use catalog_validator::validate_catalog;
use serde::Deserialize;
use serde::de::{Error as DeError, MapAccess, Visitor};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fmt,
    fs,
    path::{Component, Path, PathBuf},
};

pub mod groovy;

#[derive(Debug)]
pub struct RuntimeError(pub String);
impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for RuntimeError {}

#[derive(Debug)]
pub struct Runtime {
    manifest: Value,
    catalog: Value,
}

#[derive(Debug)]
struct UniqueChecksums(BTreeMap<String, Value>);

impl<'de> Deserialize<'de> for UniqueChecksums {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct ChecksumsVisitor;

        impl<'de> Visitor<'de> for ChecksumsVisitor {
            type Value = UniqueChecksums;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object with unique checksum file names")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut checksums = BTreeMap::new();
                while let Some((name, digest)) = map.next_entry::<String, Value>()? {
                    if checksums.insert(name.clone(), digest).is_some() {
                        return Err(M::Error::custom(format!(
                            "duplicate checksum entry '{name}'"
                        )));
                    }
                }
                Ok(UniqueChecksums(checksums))
            }
        }

        deserializer.deserialize_map(ChecksumsVisitor)
    }
}

#[derive(Deserialize)]
struct ManifestDocument {
    #[serde(flatten)]
    fields: serde_json::Map<String, Value>,
    checksums: UniqueChecksums,
}

impl ManifestDocument {
    fn into_value(self) -> Value {
        let mut manifest = self.fields;
        manifest.insert(
            "checksums".into(),
            Value::Object(self.checksums.0.into_iter().collect()),
        );
        Value::Object(manifest)
    }
}

impl Runtime {
    #[cfg(test)]
    pub(crate) fn from_validated_parts(manifest: Value, catalog: Value) -> Self {
        Self { manifest, catalog }
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, RuntimeError> {
        let path = path.as_ref();
        let manifest_path = if path.is_dir() {
            path.join("manifest.json")
        } else {
            path.to_path_buf()
        };
        let package_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
        let manifest_text = fs::read_to_string(&manifest_path)
            .map_err(|e| RuntimeError(format!("cannot read {}: {e}", manifest_path.display())))?;
        let manifest_document: ManifestDocument = serde_json::from_str(&manifest_text)
            .map_err(|e| RuntimeError(format!("invalid JSON in {}: {e}", manifest_path.display())))?;
        let manifest = manifest_document.into_value();

        let manifest_schema: Value = serde_json::from_str(include_str!(
            "../../../schemas/manifest.schema.json"
        ))
        .expect("bundled manifest schema");
        validate_schema(&manifest, &manifest_schema, "manifest")?;

        let catalog_name = manifest
            .get("catalog")
            .and_then(Value::as_str)
            .ok_or_else(|| RuntimeError("manifest.catalog must be a string".into()))?;
        let checksums = manifest
            .get("checksums")
            .and_then(Value::as_object)
            .ok_or_else(|| RuntimeError("manifest.checksums must be an object".into()))?;
        if !checksums.contains_key(catalog_name) {
            return Err(RuntimeError(format!(
                "manifest catalog '{catalog_name}' has no checksum entry"
            )));
        }

        let mut verified_files = BTreeMap::new();
        for (name, expected) in checksums {
            let expected = expected.as_str().ok_or_else(|| {
                RuntimeError(format!("checksum for '{name}' must be a SHA-256 hex string"))
            })?;
            if expected.len() != 64
                || !expected
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(RuntimeError(format!(
                    "checksum for '{name}' must be 64 lowercase hexadecimal characters"
                )));
            }

            let file_path = resolve_package_file(package_dir, name)?;
            let contents = fs::read(&file_path).map_err(|e| {
                RuntimeError(format!("cannot read checksummed package file '{name}': {e}"))
            })?;
            let actual = hex(&Sha256::digest(&contents));
            if actual != expected {
                return Err(RuntimeError(format!(
                    "checksum mismatch for package file '{name}'"
                )));
            }
            verified_files.insert(name.clone(), contents);
        }

        let catalog_bytes = verified_files.get(catalog_name).ok_or_else(|| {
            RuntimeError(format!("catalog '{catalog_name}' was not checksum-verified"))
        })?;
        let catalog: Value = serde_json::from_slice(catalog_bytes)
            .map_err(|e| RuntimeError(format!("invalid JSON in catalog '{catalog_name}': {e}")))?;

        let catalog_schema: Value =
            serde_json::from_str(include_str!("../../../schemas/catalog.schema.json"))
                .expect("bundled catalog schema");
        validate_catalog(&catalog, &catalog_schema)
            .map_err(|e| RuntimeError(format!("invalid catalog '{catalog_name}': {e}")))?;
        let expected = manifest
            .get("catalogSchemaVersion")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let actual = catalog
            .get("schemaVersion")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if expected != actual {
            return Err(RuntimeError(format!(
                "package/catalog schema version mismatch: manifest declares '{expected}', catalog is '{actual}'"
            )));
        }
        Ok(Self { manifest, catalog })
    }

    pub fn profile(&self) -> &str {
        self.manifest["profile"].as_str().unwrap_or_default()
    }
    pub fn manifest(&self) -> &Value {
        &self.manifest
    }
    pub fn catalog(&self) -> &Value {
        &self.catalog
    }
    pub fn symbols(&self) -> &[Value] {
        self.catalog["symbols"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
    pub fn sources(&self) -> &[Value] {
        self.catalog["sources"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn search(&self, query: &str, kind: Option<&str>, source: Option<&str>) -> Vec<Value> {
        let q = query.to_lowercase();
        self.symbols()
            .iter()
            .filter(|s| {
                (q.is_empty()
                    || ["name", "description", "signature", "namespace"]
                        .iter()
                        .filter_map(|k| s.get(k).and_then(Value::as_str))
                        .any(|v| v.to_lowercase().contains(&q)))
                    && kind.is_none_or(|v| s["kind"].as_str() == Some(v))
                    && source.is_none_or(|v| s["sourceId"].as_str() == Some(v))
            })
            .map(|s| self.enrich(s))
            .collect()
    }

    pub fn get(&self, name_or_id: &str) -> Result<Value, RuntimeError> {
        let matches: Vec<_> = self
            .symbols()
            .iter()
            .filter(|s| {
                s["id"].as_str() == Some(name_or_id)
                    || s["name"].as_str() == Some(name_or_id)
            })
            .collect();
        match matches.as_slice() {
            [] => Err(RuntimeError(format!(
                "symbol '{name_or_id}' was not found"
            ))),
            [symbol] => Ok(self.enrich(symbol)),
            many => Err(RuntimeError(format!(
                "symbol '{name_or_id}' is ambiguous ({} matches); use a symbol ID or filter by source",
                many.len()
            ))),
        }
    }

    pub fn examples(&self, name_or_id: &str) -> Result<Value, RuntimeError> {
        let symbol = self.get(name_or_id)?;
        Ok(serde_json::json!({
            "id":symbol["id"], "name":symbol["name"],
            "examples":symbol["examples"].as_array().cloned().unwrap_or_default(),
            "source":symbol["source"], "profile":symbol["profile"], "confidence":symbol["confidence"]
        }))
    }

    pub fn pipeline_step(&self, name: &str) -> Result<Value, RuntimeError> {
        let matches: Vec<_> = self
            .symbols()
            .iter()
            .filter(|s| {
                s["kind"].as_str() == Some("step")
                    && (s["name"].as_str() == Some(name) || s["id"].as_str() == Some(name))
            })
            .collect();
        match matches.as_slice() {
            [] => Err(RuntimeError(format!(
                "Pipeline step '{name}' was not found"
            ))),
            [symbol] => Ok(self.enrich(symbol)),
            many => Err(RuntimeError(format!(
                "Pipeline step '{name}' is ambiguous ({} matches); use a symbol ID",
                many.len()
            ))),
        }
    }

    fn enrich(&self, symbol: &Value) -> Value {
        let source_id = symbol["sourceId"].as_str();
        let source = self.sources().iter().find(|s| s["id"].as_str() == source_id);
        let mut result = symbol.clone();
        if let Some(object) = result.as_object_mut() {
            object.insert("source".into(), source.cloned().unwrap_or(Value::Null));
            object.insert("profile".into(), Value::String(self.profile().into()));
        }
        result
    }
}

fn resolve_package_file(package_dir: &Path, name: &str) -> Result<PathBuf, RuntimeError> {
    if name.is_empty() || name.contains('\\') || name.contains('\0') {
        return Err(RuntimeError(format!(
            "unsafe checksum path '{name}': expected a relative package path"
        )));
    }
    let relative = Path::new(name);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(RuntimeError(format!(
            "unsafe checksum path '{name}': absolute and traversal paths are not allowed"
        )));
    }

    let root = fs::canonicalize(package_dir).map_err(|e| {
        RuntimeError(format!(
            "cannot resolve package directory {}: {e}",
            package_dir.display()
        ))
    })?;
    let mut current = root.clone();
    for component in relative.components() {
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current).map_err(|e| {
            RuntimeError(format!("cannot access checksummed package file '{name}': {e}"))
        })?;
        if metadata.file_type().is_symlink() {
            return Err(RuntimeError(format!(
                "unsafe checksum path '{name}': symbolic links are not allowed"
            )));
        }
    }
    let resolved = fs::canonicalize(&current).map_err(|e| {
        RuntimeError(format!("cannot resolve checksummed package file '{name}': {e}"))
    })?;
    if !resolved.starts_with(&root) || !resolved.is_file() {
        return Err(RuntimeError(format!(
            "unsafe checksum path '{name}': expected a regular file inside the package"
        )));
    }
    Ok(resolved)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn validate_schema(value: &Value, schema: &Value, label: &str) -> Result<(), RuntimeError> {
    let validator = jsonschema::validator_for(schema)
        .map_err(|e| RuntimeError(format!("invalid {label} schema: {e}")))?;
    let errors: Vec<_> = validator
        .iter_errors(value)
        .map(|e| format!("{}: {e}", e.instance_path))
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(RuntimeError(format!(
            "invalid {label}:\n- {}",
            errors.join("\n- ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::Mutex,
    };

    static CWD_LOCK: Mutex<()> = Mutex::new(());

    fn runtime() -> Runtime {
        let manifest = serde_json::json!({"profile":"test"});
        let catalog: Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/catalog-valid.json"))
                .unwrap();
        Runtime { manifest, catalog }
    }

    fn write_package(directory: &Path) -> String {
        let catalog = include_str!("../../../tests/fixtures/catalog-valid.json");
        fs::write(directory.join("catalog.json"), catalog).unwrap();
        let checksum = hex(&Sha256::digest(catalog.as_bytes()));
        let manifest = serde_json::json!({
            "packageVersion": "1.0.0",
            "catalogSchemaVersion": "1.0.0",
            "profile": "test",
            "createdAt": "2026-01-01T00:00:00Z",
            "catalog": "catalog.json",
            "sourceRevisions": {},
            "provenance": {},
            "checksums": {"catalog.json": checksum},
            "toolVersions": {}
        });
        fs::write(
            directory.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        checksum
    }

    #[test]
    fn opens_a_valid_checksummed_package() {
        let directory = tempfile::tempdir().unwrap();
        write_package(directory.path());
        let loaded = Runtime::open(directory.path()).unwrap();
        assert_eq!(loaded.profile(), "test");
    }

    #[test]
    fn rejects_catalog_tampering_before_json_parsing() {
        let directory = tempfile::tempdir().unwrap();
        write_package(directory.path());
        fs::write(directory.path().join("catalog.json"), "not json").unwrap();
        let error = Runtime::open(directory.path()).unwrap_err().to_string();
        assert!(error.contains("checksum mismatch"));
    }

    #[test]
    fn rejects_missing_checksummed_files() {
        let directory = tempfile::tempdir().unwrap();
        write_package(directory.path());
        fs::remove_file(directory.path().join("catalog.json")).unwrap();
        let error = Runtime::open(directory.path()).unwrap_err().to_string();
        assert!(error.contains("cannot access checksummed package file"));
    }

    #[test]
    fn rejects_path_traversal_checksum_entries() {
        let directory = tempfile::tempdir().unwrap();
        write_package(directory.path());
        let manifest_path = directory.path().join("manifest.json");
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["checksums"]["../outside.json"] = Value::String("a".repeat(64));
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let error = Runtime::open(directory.path()).unwrap_err().to_string();
        assert!(error.contains("unsafe checksum path"));
    }

    #[test]
    fn rejects_malformed_checksum_values() {
        let directory = tempfile::tempdir().unwrap();
        write_package(directory.path());
        let manifest_path = directory.path().join("manifest.json");
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["checksums"]["catalog.json"] = Value::String("not-a-digest".into());
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let error = Runtime::open(directory.path()).unwrap_err().to_string();
        assert!(error.contains("64 lowercase hexadecimal"));
    }

    #[test]
    fn rejects_duplicate_checksum_entries() {
        let directory = tempfile::tempdir().unwrap();
        let catalog = include_str!("../../../tests/fixtures/catalog-valid.json");
        fs::write(directory.path().join("catalog.json"), catalog).unwrap();
        let checksum = hex(&Sha256::digest(catalog.as_bytes()));
        let manifest = format!(
            r#"{{"packageVersion":"1.0.0","catalogSchemaVersion":"1.0.0","profile":"test","createdAt":"2026-01-01T00:00:00Z","catalog":"catalog.json","sourceRevisions":{{}},"provenance":{{}},"checksums":{{"catalog.json":"{checksum}","catalog.json":"{checksum}"}},"toolVersions":{{}}}}"#
        );
        fs::write(directory.path().join("manifest.json"), manifest).unwrap();
        let error = Runtime::open(directory.path()).unwrap_err().to_string();
        assert!(error.contains("duplicate checksum entry"));
    }

    #[test]
    fn rejects_catalog_without_a_checksum_entry() {
        let directory = tempfile::tempdir().unwrap();
        write_package(directory.path());
        let manifest_path = directory.path().join("manifest.json");
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["checksums"] = serde_json::json!({});
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let error = Runtime::open(directory.path()).unwrap_err().to_string();
        assert!(error.contains("has no checksum entry"));
    }

    #[test]
    fn loads_bundled_schemas_from_an_unrelated_working_directory() {
        let _lock = CWD_LOCK.lock().unwrap();
        let package = tempfile::tempdir().unwrap();
        write_package(package.path());
        let unrelated = tempfile::tempdir().unwrap();
        let previous = std::env::current_dir().unwrap();
        std::env::set_current_dir(unrelated.path()).unwrap();
        let result = Runtime::open(package.path());
        std::env::set_current_dir(previous).unwrap();
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn exact_id_and_name_lookup() {
        let r = runtime();
        assert_eq!(r.get("deployApp").unwrap()["name"], "deployApp");
        assert_eq!(
            r.get("platform-lib:deployApp").unwrap()["id"],
            "platform-lib:deployApp"
        );
    }

    #[test]
    fn missing_symbol_is_actionable() {
        assert!(runtime()
            .get("missing")
            .unwrap_err()
            .to_string()
            .contains("not found"));
    }

    #[test]
    fn search_filters_by_kind_and_text() {
        assert_eq!(
            runtime()
                .search("deploy", Some("global-variable"), None)
                .len(),
            1
        );
        assert!(runtime().search("deploy", Some("step"), None).is_empty());
    }

    #[test]
    fn ambiguous_names_are_reported() {
        let mut r = runtime();
        let mut duplicate = r.catalog["symbols"][2].clone();
        duplicate["id"] = Value::String("second-deploy".into());
        r.catalog["symbols"].as_array_mut().unwrap().push(duplicate);
        assert!(r
            .get("deployApp")
            .unwrap_err()
            .to_string()
            .contains("ambiguous"));
    }
}
