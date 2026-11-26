use jsonschema::validator_for;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fmt;

#[derive(Debug)]
pub struct ValidationErrors(pub Vec<String>);

impl fmt::Display for ValidationErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, error) in self.0.iter().enumerate() {
            if index > 0 {
                writeln!(f)?;
            }
            write!(f, "- {error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationErrors {}

pub fn validate_catalog(catalog: &Value, schema: &Value) -> Result<(), ValidationErrors> {
    let validator = validator_for(schema)
        .map_err(|error| ValidationErrors(vec![format!("invalid catalog schema: {error}")]))?;

    let mut errors: Vec<String> = validator
        .iter_errors(catalog)
        .map(|error| format!("{}: {}", error.instance_path, error))
        .collect();

    if errors.is_empty() {
        validate_semantics(catalog, schema, &mut errors);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(ValidationErrors(errors))
    }
}

fn validate_semantics(catalog: &Value, schema: &Value, errors: &mut Vec<String>) {
    let sources = catalog["sources"]
        .as_array()
        .expect("schema validated sources");
    let symbols = catalog["symbols"]
        .as_array()
        .expect("schema validated symbols");
    let mut source_ids = HashSet::new();
    let mut source_kinds = HashMap::new();
    let mut symbol_ids = HashSet::new();
    let symbol_fields = schema
        .pointer("/$defs/symbol/properties")
        .and_then(Value::as_object)
        .expect("schema validated symbol properties");

    for (index, source) in sources.iter().enumerate() {
        let id = source["id"].as_str().expect("schema validated source id");
        if !source_ids.insert(id) {
            errors.push(format!("/sources/{index}/id: duplicate source id '{id}'"));
        }
        source_kinds.insert(
            id.to_owned(),
            source["kind"].as_str().unwrap_or_default().to_owned(),
        );
        if let Some(url) = source["url"].as_str() {
            if url_has_credentials(url) {
                errors.push(format!(
                    "/sources/{index}/url: URLs must not contain embedded credentials or secret query parameters"
                ));
            }
        }
        reject_secret_fields(source, &format!("/sources/{index}"), errors);
    }

    if let Some(coverage) = catalog["coverage"].as_array() {
        let mut coverage_keys = HashSet::new();
        for (index, entry) in coverage.iter().enumerate() {
            let source_id = entry["sourceId"]
                .as_str()
                .expect("schema validated coverage source id");
            let surface = entry["surface"]
                .as_str()
                .expect("schema validated coverage surface");
            if !source_ids.contains(source_id) {
                errors.push(format!(
                    "/coverage/{index}/sourceId: unknown source id '{source_id}'"
                ));
            }
            if !coverage_keys.insert((source_id, surface)) {
                errors.push(format!(
                    "/coverage/{index}: duplicate coverage entry for source '{source_id}' and surface '{surface}'"
                ));
            }
            reject_secret_fields(entry, &format!("/coverage/{index}"), errors);
        }
    }

    if let Some(controller) = catalog.get("controller").filter(|value| !value.is_null()) {
        if let Some(base_url) = controller["identity"]["baseUrl"].as_str() {
            if url_has_credentials(base_url) {
                errors.push(
                    "/controller/identity/baseUrl: URLs must not contain embedded credentials or secret query parameters"
                        .to_owned(),
                );
            }
        }
        let mut plugin_names = HashSet::new();
        for (index, plugin) in controller["plugins"]
            .as_array()
            .expect("schema validated plugins")
            .iter()
            .enumerate()
        {
            let source_id = plugin["sourceId"]
                .as_str()
                .expect("schema validated plugin source id");
            if !source_ids.contains(source_id) {
                errors.push(format!(
                    "/controller/plugins/{index}/sourceId: unknown source id '{source_id}'"
                ));
            } else if !sources
                .iter()
                .any(|source| source["id"] == source_id && source["kind"] == "plugin")
            {
                errors.push(format!(
                    "/controller/plugins/{index}/sourceId: source '{source_id}' is not a plugin source"
                ));
            }
            let short_name = plugin["shortName"]
                .as_str()
                .expect("schema validated plugin short name");
            if !plugin_names.insert(short_name) {
                errors.push(format!(
                    "/controller/plugins/{index}/shortName: duplicate plugin short name '{short_name}'"
                ));
            }
        }
        reject_secret_fields(controller, "/controller", errors);
    }

    for (index, symbol) in symbols.iter().enumerate() {
        let id = symbol["id"].as_str().expect("schema validated symbol id");
        if !symbol_ids.insert(id) {
            errors.push(format!("/symbols/{index}/id: duplicate symbol id '{id}'"));
        }
        let source_id = symbol["sourceId"]
            .as_str()
            .expect("schema validated symbol source id");
        if !source_ids.contains(source_id) {
            errors.push(format!(
                "/symbols/{index}/sourceId: unknown source id '{source_id}'"
            ));
        }

        let mut parameter_names = HashSet::new();
        if let Some(parameters) = symbol["parameters"].as_array() {
            for (parameter_index, parameter) in parameters.iter().enumerate() {
                if let Some(name) = parameter["name"].as_str() {
                    if !parameter_names.insert(name) {
                        errors.push(format!(
                            "/symbols/{index}/parameters/{parameter_index}/name: duplicate parameter name '{name}'"
                        ));
                    }
                }
            }
        }
        if let Some(availability) = symbol.get("availability") {
            if let Some(plugin_ids) = availability["pluginSourceIds"].as_array() {
                for (plugin_index, plugin_id) in plugin_ids.iter().enumerate() {
                    let plugin_id = plugin_id
                        .as_str()
                        .expect("schema validated plugin source id");
                    if !source_ids.contains(plugin_id) {
                        errors.push(format!(
                            "/symbols/{index}/availability/pluginSourceIds/{plugin_index}: unknown source id '{plugin_id}'"
                        ));
                    } else if !sources
                        .iter()
                        .any(|source| source["id"] == plugin_id && source["kind"] == "plugin")
                    {
                        errors.push(format!(
                            "/symbols/{index}/availability/pluginSourceIds/{plugin_index}: source '{plugin_id}' is not a plugin source"
                        ));
                    }
                }
            }
        }
        if let Some(field_evidence) = symbol["fieldEvidence"].as_object() {
            for field in field_evidence.keys() {
                let path = format!("/symbols/{index}/fieldEvidence/{field}");
                if !symbol_fields.contains_key(field)
                    || matches!(field.as_str(), "fieldEvidence" | "evidence")
                {
                    errors.push(format!(
                        "{path}: field is not a schema-defined symbol attribute"
                    ));
                } else if symbol.get(field).is_none() {
                    errors.push(format!(
                        "{path}: evidence refers to a field that is absent from this symbol"
                    ));
                }
            }
        }
        if let Some(evidence) = symbol["evidence"].as_array() {
            for (evidence_index, item) in evidence.iter().enumerate() {
                let source_id = item["sourceId"]
                    .as_str()
                    .expect("schema validated evidence source id");
                if !source_ids.contains(source_id) {
                    errors.push(format!(
                        "/symbols/{index}/evidence/{evidence_index}/sourceId: unknown source id '{source_id}'"
                    ));
                    continue;
                }
                let evidence_kind = item["kind"]
                    .as_str()
                    .expect("schema validated evidence kind");
                let source_kind = source_kinds
                    .get(source_id)
                    .map(String::as_str)
                    .unwrap_or_default();
                if !evidence_compatible(evidence_kind, source_kind) {
                    errors.push(format!(
                        "/symbols/{index}/evidence/{evidence_index}/kind: evidence kind '{evidence_kind}' is incompatible with source '{source_id}' of kind '{source_kind}'"
                    ));
                }
            }
        }
        reject_secret_fields(symbol, &format!("/symbols/{index}"), errors);
    }

    validate_coverage_consistency(catalog, &source_kinds, errors);
}

/// Reconcile manifest source metadata with the catalog records it describes.
/// Unknown keys inside each provenance record are intentionally preserved for forward compatibility.
pub fn validate_package_metadata(
    catalog: &Value,
    manifest: &Value,
) -> Result<(), ValidationErrors> {
    let mut errors = Vec::new();
    let Some(sources) = catalog["sources"].as_array() else {
        return Err(ValidationErrors(vec![
            "/sources: catalog sources must be an array".into(),
        ]));
    };
    let Some(revisions) = manifest["sourceRevisions"].as_object() else {
        return Err(ValidationErrors(vec![
            "/sourceRevisions: manifest sourceRevisions must be an object".into(),
        ]));
    };
    let Some(provenance) = manifest["provenance"].as_object() else {
        return Err(ValidationErrors(vec![
            "/provenance: manifest provenance must be an object".into(),
        ]));
    };

    let mut source_ids = HashSet::new();
    for (index, source) in sources.iter().enumerate() {
        let Some(id) = source["id"].as_str() else {
            continue;
        };
        source_ids.insert(id.to_owned());
        let expected_revision = source["revision"].as_str().unwrap_or_default();
        match revisions.get(id).and_then(Value::as_str) {
            Some(actual) if actual == expected_revision => {}
            Some(actual) => errors.push(format!(
                "/sourceRevisions/{id}: revision '{actual}' does not match catalog source revision '{expected_revision}' at /sources/{index}/revision"
            )),
            None => errors.push(format!(
                "/sourceRevisions/{id}: missing revision entry for catalog source at /sources/{index}"
            )),
        }

        match provenance.get(id).and_then(Value::as_object) {
            None => errors.push(format!(
                "/provenance/{id}: missing provenance record for catalog source at /sources/{index}"
            )),
            Some(record) => {
                for field in ["name", "kind", "revision"] {
                    let expected = source[field].as_str().unwrap_or_default();
                    match record.get(field).and_then(Value::as_str) {
                        Some(actual) if actual == expected => {}
                        Some(actual) => errors.push(format!(
                            "/provenance/{id}/{field}: value '{actual}' does not match catalog source value '{expected}'"
                        )),
                        None => errors.push(format!(
                            "/provenance/{id}/{field}: required source provenance field is missing"
                        )),
                    }
                }
                let expected_url = source.get("url").and_then(Value::as_str);
                let actual_url = record.get("url").and_then(Value::as_str);
                match expected_url {
                    Some(expected) if actual_url != Some(expected) => errors.push(format!(
                        "/provenance/{id}/url: URL does not match catalog source /sources/{index}/url"
                    )),
                    None if record.get("url").is_some_and(|value| !value.is_null()) => {
                        errors.push(format!(
                            "/provenance/{id}/url: provenance URL is present but catalog source /sources/{index} has no URL"
                        ))
                    }
                    _ => {}
                }
            }
        }
    }

    for id in revisions.keys() {
        if !source_ids.contains(id) {
            errors.push(format!(
                "/sourceRevisions/{id}: revision entry refers to no catalog source"
            ));
        }
    }
    for id in provenance.keys() {
        if !source_ids.contains(id) {
            errors.push(format!(
                "/provenance/{id}: provenance entry refers to no catalog source"
            ));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(ValidationErrors(errors))
    }
}

fn evidence_compatible(evidence_kind: &str, source_kind: &str) -> bool {
    match evidence_kind {
        "controller-observed" => {
            matches!(source_kind, "jenkins-controller" | "plugin" | "jenkins-core")
        }
        "source-scanned" => {
            matches!(source_kind, "shared-library" | "plugin" | "jenkins-core")
        }
        "static-model" => matches!(source_kind, "jenkins-controller" | "jenkins-core"),
        "inferred" => matches!(
            source_kind,
            "jenkins-controller" | "plugin" | "shared-library" | "jenkins-core"
        ),
        _ => false,
    }
}

fn coverage_symbol_kinds(surface: &str) -> Option<&'static [&'static str]> {
    let surface = surface.to_ascii_lowercase().replace('_', "-");
    if surface.contains("pipeline-step") || surface == "steps" {
        Some(&["step"])
    } else if surface.contains("declarative-directive") {
        Some(&["declarative-directive"])
    } else if surface.contains("shared-library-var") || surface.contains("global-variable") {
        Some(&["global-variable"])
    } else if surface.contains("shared-library-src") {
        Some(&["method", "class", "property"])
    } else if surface.contains("method") {
        Some(&["method"])
    } else if surface.contains("class") {
        Some(&["class"])
    } else if surface.contains("property") {
        Some(&["property"])
    } else {
        None
    }
}

fn coverage_source_kinds(surface: &str) -> Option<&'static [&'static str]> {
    let surface = surface.to_ascii_lowercase().replace('_', "-");
    if surface.contains("pipeline-step") || surface == "steps" {
        Some(&["jenkins-controller", "plugin", "jenkins-core"])
    } else if surface.contains("declarative-directive") {
        Some(&["jenkins-controller", "jenkins-core"])
    } else if surface.contains("shared-library-var") || surface.contains("shared-library-src") {
        Some(&["shared-library"])
    } else {
        None
    }
}

fn validate_coverage_consistency(
    catalog: &Value,
    source_kinds: &HashMap<String, String>,
    errors: &mut Vec<String>,
) {
    let Some(coverage) = catalog["coverage"].as_array() else {
        return;
    };
    let Some(symbols) = catalog["symbols"].as_array() else {
        return;
    };
    for (coverage_index, entry) in coverage.iter().enumerate() {
        let status = entry["status"].as_str().unwrap_or_default();
        let source_id = entry["sourceId"].as_str().unwrap_or_default();
        let surface = entry["surface"].as_str().unwrap_or_default();
        if let Some(allowed) = coverage_source_kinds(surface) {
            let actual = source_kinds
                .get(source_id)
                .map(String::as_str)
                .unwrap_or_default();
            if !allowed.contains(&actual) {
                errors.push(format!(
                    "/coverage/{coverage_index}/surface: surface '{surface}' is incompatible with source '{source_id}' of kind '{actual}'"
                ));
                continue;
            }
        }
        if status == "inspected" {
            continue;
        }
        let Some(kinds) = coverage_symbol_kinds(surface) else {
            continue;
        };
        for (symbol_index, symbol) in symbols.iter().enumerate() {
            if !kinds.contains(&symbol["kind"].as_str().unwrap_or_default()) {
                continue;
            }
            let symbol_source_matches = symbol["sourceId"].as_str() == Some(source_id);
            let evidence_source_matches = symbol["evidence"]
                .as_array()
                .is_some_and(|evidence| {
                    evidence
                        .iter()
                        .any(|item| item["sourceId"].as_str() == Some(source_id))
                });
            if symbol_source_matches || evidence_source_matches {
                errors.push(format!(
                    "/coverage/{coverage_index}: status '{status}' for surface '{surface}' contradicts symbol /symbols/{symbol_index} or its evidence from source '{source_id}'"
                ));
                break;
            }
        }
    }
}

fn url_has_credentials(value: &str) -> bool {
    let Some((_, remainder)) = value.split_once("://") else {
        return false;
    };
    let authority = remainder.split('/').next().unwrap_or_default();
    if authority.contains('@') {
        return true;
    }
    value.split_once('?').is_some_and(|(_, query)| {
        query.split('&').any(|part| {
            let key = part
                .split('=')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            [
                "token",
                "api_key",
                "apikey",
                "password",
                "secret",
                "authorization",
                "access_token",
            ]
            .iter()
            .any(|sensitive| key.contains(sensitive))
        })
    })
}

fn reject_secret_fields(value: &Value, path: &str, errors: &mut Vec<String>) {
    match value {
        Value::Object(fields) => {
            for (key, child) in fields {
                let normalized = key.to_ascii_lowercase().replace(['_', '-'], "");
                if [
                    "password",
                    "token",
                    "apitoken",
                    "authorization",
                    "secret",
                    "credentials",
                    "clientsecret",
                ]
                .contains(&normalized.as_str())
                {
                    errors.push(format!(
                        "{path}/{key}: secret-bearing fields are not permitted in catalogs"
                    ));
                } else {
                    reject_secret_fields(child, &format!("{path}/{key}"), errors);
                }
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                reject_secret_fields(child, &format!("{path}/{index}"), errors);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Value {
        serde_json::from_str(include_str!("../../../schemas/catalog.schema.json")).unwrap()
    }

    fn valid_catalog() -> Value {
        serde_json::from_str(include_str!("../../../tests/fixtures/catalog-valid.json")).unwrap()
    }

    #[test]
    fn accepts_valid_catalog() {
        assert!(validate_catalog(&valid_catalog(), &schema()).is_ok());
    }

    #[test]
    fn rejects_missing_required_fields() {
        let errors = validate_catalog(&json!({"schemaVersion": "1.0.0"}), &schema()).unwrap_err();
        assert!(errors.to_string().contains("catalogId"));
    }

    #[test]
    fn rejects_unknown_symbol_source() {
        let mut invalid = valid_catalog();
        invalid["symbols"][0]["sourceId"] = json!("missing-source");
        assert!(validate_catalog(&invalid, &schema())
            .unwrap_err()
            .to_string()
            .contains("unknown source id"));
    }

    #[test]
    fn rejects_duplicate_symbol_ids() {
        let mut invalid = valid_catalog();
        let duplicate = invalid["symbols"][0].clone();
        invalid["symbols"].as_array_mut().unwrap().push(duplicate);
        assert!(validate_catalog(&invalid, &schema())
            .unwrap_err()
            .to_string()
            .contains("duplicate symbol id"));
    }

    #[test]
    fn rejects_duplicate_source_ids() {
        let mut invalid = valid_catalog();
        let duplicate = invalid["sources"][0].clone();
        invalid["sources"].as_array_mut().unwrap().push(duplicate);
        assert!(validate_catalog(&invalid, &schema())
            .unwrap_err()
            .to_string()
            .contains("duplicate source id"));
    }

    #[test]
    fn rejects_duplicate_parameter_names() {
        let mut invalid = valid_catalog();
        invalid["symbols"][0]["parameters"] = json!([{"name":"script"},{"name":"script"}]);
        assert!(validate_catalog(&invalid, &schema())
            .unwrap_err()
            .to_string()
            .contains("duplicate parameter name"));
    }

    #[test]
    fn rejects_secret_fields_and_credential_urls() {
        let mut invalid = valid_catalog();
        invalid["sources"][0]["metadata"] = json!({"api_token":"must-not-leak"});
        let errors = validate_catalog(&invalid, &schema())
            .unwrap_err()
            .to_string();
        assert!(errors.contains("secret-bearing fields"));
        assert!(!errors.contains("must-not-leak"));
    }

    #[test]
    fn rejects_urls_with_embedded_credentials() {
        let mut invalid = valid_catalog();
        invalid["sources"][0]["url"] = json!("https://user:password@example.test/repo");
        assert!(validate_catalog(&invalid, &schema())
            .unwrap_err()
            .to_string()
            .contains("embedded credentials"));
    }

    #[test]
    fn rejects_coverage_for_unknown_source() {
        let mut invalid = valid_catalog();
        invalid["coverage"] = json!([{
            "sourceId":"missing-source",
            "surface":"pipeline-steps",
            "status":"not-collected"
        }]);
        assert!(validate_catalog(&invalid, &schema())
            .unwrap_err()
            .to_string()
            .contains("/coverage/0/sourceId"));
    }

    #[test]
    fn rejects_duplicate_coverage_for_the_same_source_surface() {
        let mut invalid = valid_catalog();
        invalid["coverage"] = json!([
            {"sourceId":"controller","surface":"pipeline-steps","status":"inspected"},
            {"sourceId":"controller","surface":"pipeline-steps","status":"unavailable"}
        ]);
        assert!(validate_catalog(&invalid, &schema())
            .unwrap_err()
            .to_string()
            .contains("duplicate coverage entry"));
    }

    #[test]
    fn accepts_field_level_evidence_without_changing_symbol_confidence() {
        let mut catalog = valid_catalog();
        catalog["symbols"][0]["fieldEvidence"] = json!({
            "name":"verified",
            "parameters":"unknown",
            "description":"inferred"
        });
        assert!(validate_catalog(&catalog, &schema()).is_ok());
    }

    #[test]
    fn accepts_controller_runtime_snapshot_fixture() {
        assert!(validate_catalog(&valid_catalog(), &schema()).is_ok());
    }

    #[test]
    fn accepts_legacy_catalog_without_runtime_snapshot() {
        let legacy: Value = serde_json::from_str(include_str!(
            "../../../crates/knowledge-builder/tests/fixtures/catalog.json"
        ))
        .unwrap();
        assert!(validate_catalog(&legacy, &schema()).is_ok());
    }

    #[test]
    fn rejects_unknown_execution_constraint_values() {
        let mut invalid = valid_catalog();
        invalid["symbols"][0]["executionConstraints"]["sandbox"] = json!("maybe");
        assert!(validate_catalog(&invalid, &schema()).is_err());
    }

    #[test]
    fn rejects_controller_plugin_referencing_non_plugin_source() {
        let mut invalid = valid_catalog();
        invalid["controller"]["plugins"][0]["sourceId"] = json!("controller");
        assert!(validate_catalog(&invalid, &schema())
            .unwrap_err()
            .to_string()
            .contains("is not a plugin source"));
    }

    #[test]
    fn rejects_credentials_in_controller_base_url() {
        let mut invalid = valid_catalog();
        invalid["controller"]["identity"]["baseUrl"] =
            json!("https://user:password@jenkins.example.invalid/");
        assert!(validate_catalog(&invalid, &schema())
            .unwrap_err()
            .to_string()
            .contains("controller/identity/baseUrl"));
    }

    #[test]
    fn rejects_availability_reference_to_non_plugin_source() {
        let mut invalid = valid_catalog();
        invalid["symbols"][0]["availability"]["pluginSourceIds"] = json!(["controller"]);
        assert!(validate_catalog(&invalid, &schema())
            .unwrap_err()
            .to_string()
            .contains("is not a plugin source"));
    }

    #[test]
    fn rejects_symbol_evidence_with_unknown_source() {
        let mut invalid = valid_catalog();
        invalid["symbols"][0]["evidence"][0]["sourceId"] = json!("missing-source");
        assert!(validate_catalog(&invalid, &schema())
            .unwrap_err()
            .to_string()
            .contains("unknown source id"));
    }

    #[test]
    fn rejects_duplicate_controller_plugin_short_names() {
        let mut invalid = valid_catalog();
        let duplicate = invalid["controller"]["plugins"][0].clone();
        invalid["controller"]["plugins"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert!(validate_catalog(&invalid, &schema())
            .unwrap_err()
            .to_string()
            .contains("duplicate plugin short name"));
    }

    #[test]
    fn allows_same_name_symbols_from_distinct_sources() {
        let mut catalog = valid_catalog();
        let mut source = catalog["sources"][0].clone();
        source["id"] = json!("another-source");
        source["revision"] = json!("revision-2");
        catalog["sources"].as_array_mut().unwrap().push(source);
        let mut symbol = catalog["symbols"][0].clone();
        symbol["id"] = json!("another-symbol");
        symbol["sourceId"] = json!("another-source");
        catalog["symbols"].as_array_mut().unwrap().push(symbol);
        assert!(validate_catalog(&catalog, &schema()).is_ok());
    }
    fn manifest_for(catalog: &Value) -> Value {
        let mut source_revisions = serde_json::Map::new();
        let mut provenance = serde_json::Map::new();
        for source in catalog["sources"].as_array().unwrap() {
            let id = source["id"].as_str().unwrap();
            source_revisions.insert(id.into(), source["revision"].clone());
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
        json!({"sourceRevisions": source_revisions, "provenance": provenance})
    }

    #[test]
    fn accepts_manifest_metadata_that_matches_catalog_sources() {
        let catalog = valid_catalog();
        assert!(validate_package_metadata(&catalog, &manifest_for(&catalog)).is_ok());
    }

    #[test]
    fn rejects_missing_and_extra_source_revision_entries() {
        let catalog = valid_catalog();
        let mut manifest = manifest_for(&catalog);
        manifest["sourceRevisions"]
            .as_object_mut()
            .unwrap()
            .remove("platform-lib");
        assert!(validate_package_metadata(&catalog, &manifest)
            .unwrap_err()
            .to_string()
            .contains("/sourceRevisions/platform-lib: missing revision entry"));

        let mut manifest = manifest_for(&catalog);
        manifest["sourceRevisions"]["ghost"] = json!("revision-1");
        assert!(validate_package_metadata(&catalog, &manifest)
            .unwrap_err()
            .to_string()
            .contains("/sourceRevisions/ghost: revision entry refers to no catalog source"));
    }

    #[test]
    fn rejects_mismatched_source_revisions() {
        let catalog = valid_catalog();
        let mut manifest = manifest_for(&catalog);
        manifest["sourceRevisions"]["platform-lib"] = json!("wrong-revision");
        assert!(validate_package_metadata(&catalog, &manifest)
            .unwrap_err()
            .to_string()
            .contains("does not match catalog source revision"));
    }

    #[test]
    fn rejects_missing_extra_and_mismatched_provenance_records() {
        let catalog = valid_catalog();
        let mut manifest = manifest_for(&catalog);
        manifest["provenance"]
            .as_object_mut()
            .unwrap()
            .remove("controller");
        assert!(validate_package_metadata(&catalog, &manifest)
            .unwrap_err()
            .to_string()
            .contains("/provenance/controller: missing provenance record"));

        let mut manifest = manifest_for(&catalog);
        manifest["provenance"]["ghost"] = json!({"name":"ghost","kind":"plugin","revision":"1"});
        assert!(validate_package_metadata(&catalog, &manifest)
            .unwrap_err()
            .to_string()
            .contains("/provenance/ghost: provenance entry refers to no catalog source"));

        let mut manifest = manifest_for(&catalog);
        manifest["provenance"]["platform-lib"]["kind"] = json!("plugin");
        assert!(validate_package_metadata(&catalog, &manifest)
            .unwrap_err()
            .to_string()
            .contains("/provenance/platform-lib/kind"));
    }

    #[test]
    fn permits_unknown_forward_compatible_provenance_metadata() {
        let catalog = valid_catalog();
        let mut manifest = manifest_for(&catalog);
        manifest["provenance"]["platform-lib"]["futureMetadata"] = json!({"registry":"internal"});
        assert!(validate_package_metadata(&catalog, &manifest).is_ok());
    }

    #[test]
    fn rejects_field_evidence_for_unknown_schema_attributes() {
        let mut invalid = valid_catalog();
        invalid["symbols"][0]["fieldEvidence"] = json!({"madeUpField":"verified"});
        let error = validate_catalog(&invalid, &schema())
            .unwrap_err()
            .to_string();
        assert!(error.contains("/symbols/0/fieldEvidence/madeUpField"));
        assert!(error.contains("not a schema-defined symbol attribute"));
    }

    #[test]
    fn rejects_field_evidence_for_attributes_absent_from_the_symbol() {
        let mut invalid = valid_catalog();
        invalid["symbols"][0]["fieldEvidence"] = json!({"examples":"verified"});
        let error = validate_catalog(&invalid, &schema()).unwrap_err().to_string();
        assert!(error.contains("/symbols/0/fieldEvidence/examples"));
        assert!(error.contains("field is absent"));
    }

    #[test]
    fn enforces_evidence_kind_and_source_kind_compatibility() {
        let mut valid = valid_catalog();
        valid["symbols"][0]["evidence"][0]["sourceId"] = json!("platform-lib");
        valid["symbols"][0]["evidence"][0]["kind"] = json!("source-scanned");
        assert!(validate_catalog(&valid, &schema()).is_ok());

        let mut invalid = valid_catalog();
        invalid["symbols"][0]["evidence"][0]["sourceId"] = json!("platform-lib");
        let error = validate_catalog(&invalid, &schema()).unwrap_err().to_string();
        assert!(error.contains("/symbols/0/evidence/0/kind"));
        assert!(error.contains(
            "incompatible with source 'platform-lib' of kind 'shared-library'"
        ));
    }

    #[test]
    fn rejects_coverage_claims_contradicted_by_catalog_records() {
        let mut invalid = valid_catalog();
        invalid["coverage"] = json!([{
            "sourceId":"controller",
            "surface":"pipeline-steps",
            "status":"not-collected"
        }]);
        let error = validate_catalog(&invalid, &schema()).unwrap_err().to_string();
        assert!(error.contains("/coverage/0"));
        assert!(error.contains("contradicts symbol /symbols/0"));
    }

    #[test]
    fn allows_empty_inspected_surfaces_and_unknown_forward_compatible_surfaces() {
        let mut catalog = valid_catalog();
        catalog["coverage"] = json!([
            {"sourceId":"platform-lib","surface":"shared-library-vars","status":"inspected"},
            {"sourceId":"controller","surface":"future-surface-v2","status":"unavailable"}
        ]);
        assert!(validate_catalog(&catalog, &schema()).is_ok());
    }

    #[test]
    fn rejects_coverage_surface_incompatible_with_source_kind() {
        let mut catalog = valid_catalog();
        catalog["coverage"] = json!([{
            "sourceId":"controller",
            "surface":"shared-library-vars",
            "status":"inspected"
        }]);
        let error = validate_catalog(&catalog, &schema()).unwrap_err().to_string();
        assert!(error.contains(
            "surface 'shared-library-vars' is incompatible with source 'controller' of kind 'jenkins-controller'"
        ));
    }

    #[test]
    fn rejects_field_evidence_metadata_fields() {
        let mut invalid = valid_catalog();
        invalid["symbols"][0]["fieldEvidence"] = json!({"fieldEvidence":"verified"});
        let error = validate_catalog(&invalid, &schema())
            .unwrap_err()
            .to_string();
        assert!(error.contains("not a schema-defined symbol attribute"));
    }

}
