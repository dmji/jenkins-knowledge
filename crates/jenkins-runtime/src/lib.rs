use catalog_validator::validate_catalog;
use jsonschema::validator_for;
use serde_json::Value;
use std::{fs, path::Path};

#[derive(Debug)]
pub struct RuntimeError(pub String);

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for RuntimeError {}

pub struct Runtime {
    manifest: Value,
    catalog: Value,
}

impl Runtime {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RuntimeError> {
        let path = path.as_ref();
        let manifest_path = if path.is_dir() {
            path.join("manifest.json")
        } else {
            path.to_path_buf()
        };
        let package_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
        let manifest: Value = read_json(&manifest_path)?;
        let manifest_schema: Value = read_json(Path::new("schemas/manifest.schema.json"))?;
        validate_schema(&manifest, &manifest_schema, "manifest")?;
        let catalog_name = manifest
            .get("catalog")
            .and_then(Value::as_str)
            .ok_or_else(|| RuntimeError("manifest.catalog must be a string".into()))?;
        let catalog_path = package_dir.join(catalog_name);
        let catalog: Value = read_json(&catalog_path)?;
        let catalog_schema: Value = read_json(Path::new("schemas/catalog.schema.json"))?;
        validate_catalog(&catalog, &catalog_schema).map_err(|e| {
            RuntimeError(format!("invalid catalog {}: {e}", catalog_path.display()))
        })?;
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
                s["id"].as_str() == Some(name_or_id) || s["name"].as_str() == Some(name_or_id)
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
            "id": symbol["id"], "name": symbol["name"],
            "examples": symbol["examples"].as_array().cloned().unwrap_or_default(),
            "source": symbol["source"], "confidence": symbol["confidence"]
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
        let source = self
            .sources()
            .iter()
            .find(|s| s["id"].as_str() == source_id);
        let mut result = symbol.clone();
        if let Some(object) = result.as_object_mut() {
            object.insert("source".into(), source.cloned().unwrap_or(Value::Null));
            object.insert("profile".into(), Value::String(self.profile().into()));
        }
        result
    }
}

fn read_json(path: &Path) -> Result<Value, RuntimeError> {
    let content = fs::read_to_string(path)
        .map_err(|e| RuntimeError(format!("cannot read {}: {e}", path.display())))?;
    serde_json::from_str(&content)
        .map_err(|e| RuntimeError(format!("invalid JSON in {}: {e}", path.display())))
}

fn validate_schema(value: &Value, schema: &Value, label: &str) -> Result<(), RuntimeError> {
    let validator =
        validator_for(schema).map_err(|e| RuntimeError(format!("invalid {label} schema: {e}")))?;
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

    fn runtime() -> Runtime {
        let manifest = serde_json::json!({"profile":"test"});
        let catalog: Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/catalog-valid.json"))
                .unwrap();
        Runtime { manifest, catalog }
    }

    #[test]
    fn exact_id_and_name_lookup() {
        let r = runtime();
        assert_eq!(r.get("deployApp").unwrap()["name"], "deployApp");
        assert_eq!(
            r.get("platform-lib:vars:deployApp").unwrap()["id"],
            "platform-lib:vars:deployApp"
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
        let results = runtime().search("deploy", Some("global-variable"), None);
        assert_eq!(results.len(), 1);
        assert!(runtime().search("deploy", Some("step"), None).is_empty());
    }

    #[test]
    fn ambiguous_names_are_reported() {
        let mut r = runtime();
        let mut duplicate = r.catalog["symbols"][0].clone();
        duplicate["id"] = Value::String("second-deploy".into());
        r.catalog["symbols"].as_array_mut().unwrap().push(duplicate);
        assert!(r
            .get("deployApp")
            .unwrap_err()
            .to_string()
            .contains("ambiguous"));
    }
}
