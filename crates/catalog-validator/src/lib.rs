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


/// Return a lowercase SHA-256 digest with an explicit algorithm prefix.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    format!("sha256:{}", digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>())
}

/// Add deterministic source fingerprints and a catalog snapshot ID.
/// Collection timestamps remain available for diagnostics but do not participate in identity.
pub fn stamp_catalog_identity(catalog: &mut Value) -> Result<(), String> {
    canonicalize_arrays(catalog);
    let source_ids = catalog["sources"].as_array()
        .ok_or_else(|| "catalog sources must be an array".to_owned())?
        .iter().filter_map(|source| source["id"].as_str().map(str::to_owned)).collect::<Vec<_>>();
    for source_id in source_ids {
        let fingerprint = source_fingerprint(catalog, &source_id)?;
        if let Some(source) = catalog["sources"].as_array_mut().and_then(|sources| {
            sources.iter_mut().find(|source| source["id"].as_str() == Some(&source_id))
        }) {
            source["fingerprint"] = Value::String(fingerprint);
        }
    }
    catalog["snapshotId"] = Value::String(catalog_snapshot_id(catalog)?);
    Ok(())
}

pub fn source_fingerprint(catalog: &Value, source_id: &str) -> Result<String, String> {
    let source = catalog["sources"].as_array()
        .and_then(|sources| sources.iter().find(|source| source["id"].as_str() == Some(source_id)))
        .ok_or_else(|| format!("unknown source id '{source_id}'"))?;
    let mut source = source.clone();
    if let Some(object) = source.as_object_mut() { object.remove("fingerprint"); }
    let mut symbols = catalog["symbols"].as_array().cloned().unwrap_or_default().into_iter()
        .filter(|symbol| symbol["sourceId"].as_str() == Some(source_id)).collect::<Vec<_>>();
    symbols.sort_by_key(|symbol| symbol["id"].as_str().unwrap_or_default().to_owned());
    let mut coverage = catalog["coverage"].as_array().cloned().unwrap_or_default().into_iter()
        .filter(|entry| entry["sourceId"].as_str() == Some(source_id)).collect::<Vec<_>>();
    coverage.sort_by_key(|entry| (entry["surface"].as_str().unwrap_or_default().to_owned(), entry["status"].as_str().unwrap_or_default().to_owned()));
    let mut controller_observation = Value::Null;
    if let Some(controller) = catalog.get("controller") {
        if controller["identity"]["id"].as_str() == Some(source_id) {
            controller_observation = controller.clone();
        } else if let Some(plugins) = controller["plugins"].as_array() {
            let related = plugins.iter().filter(|plugin| plugin["sourceId"].as_str() == Some(source_id)).cloned().collect::<Vec<_>>();
            if !related.is_empty() { controller_observation = serde_json::json!({"plugins": related}); }
        }
    }
    let mut canonical = serde_json::json!({"source":source,"symbols":symbols,"coverage":coverage,"controllerObservation":controller_observation});
    strip_volatile_timestamps(&mut canonical);
    canonicalize_arrays(&mut canonical);
    let bytes = serde_json::to_vec(&canonical).map_err(|_| "cannot serialize source fingerprint input".to_owned())?;
    Ok(sha256_hex(&bytes))
}

pub fn catalog_snapshot_id(catalog: &Value) -> Result<String, String> {
    let mut canonical = catalog.clone();
    if let Some(object) = canonical.as_object_mut() { object.remove("snapshotId"); }
    strip_volatile_timestamps(&mut canonical);
    canonicalize_arrays(&mut canonical);
    let bytes = serde_json::to_vec(&canonical).map_err(|_| "cannot serialize catalog snapshot input".to_owned())?;
    Ok(sha256_hex(&bytes))
}

fn strip_volatile_timestamps(value: &mut Value) {
    if let Some(object) = value.as_object_mut() {
        for key in ["generatedAt","startedAt","completedAt","retrievedAt","collectedAt","createdAt"] { object.remove(key); }
        for child in object.values_mut() { strip_volatile_timestamps(child); }
    } else if let Some(array) = value.as_array_mut() {
        for child in array { strip_volatile_timestamps(child); }
    }
}

fn canonicalize_arrays(value: &mut Value) {
    if let Some(object) = value.as_object_mut() {
        for (key, child) in object.iter_mut() {
            canonicalize_arrays(child);
            if let Some(array) = child.as_array_mut() {
                match key.as_str() {
                    "sources" => array.sort_by_key(|item| item["id"].as_str().unwrap_or_default().to_owned()),
                    "symbols" => array.sort_by_key(|item| item["id"].as_str().unwrap_or_default().to_owned()),
                    "coverage" => array.sort_by_key(|item| (item["sourceId"].as_str().unwrap_or_default().to_owned(), item["surface"].as_str().unwrap_or_default().to_owned())),
                    "plugins" => array.sort_by_key(|item| item["shortName"].as_str().unwrap_or_default().to_owned()),
                    "capabilities" => array.sort_by_key(|item| item["name"].as_str().unwrap_or_default().to_owned()),
                    "dependencies" => array.sort_by_key(|item| item["shortName"].as_str().unwrap_or_default().to_owned()),
                    "evidence" => array.sort_by_key(|item| (item["sourceId"].as_str().unwrap_or_default().to_owned(), item["kind"].as_str().unwrap_or_default().to_owned(), item["provenance"].as_str().unwrap_or_default().to_owned())),
                    "extensionRegistries" => array.sort_by_key(|item| item["name"].as_str().unwrap_or_default().to_owned()),
                    "items" => array.sort_by_key(|item| (item["name"].as_str().unwrap_or_default().to_owned(), item["className"].as_str().unwrap_or_default().to_owned())),
                    _ => {}
                }
            }
        }
    } else if let Some(array) = value.as_array_mut() {
        for child in array { canonicalize_arrays(child); }
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
        }
    }

    if let Some(controller) = catalog.get("controller").filter(|value| !value.is_null()) {
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
    }

    for (index, source) in sources.iter().enumerate() {
        if let Some(actual) = source.get("fingerprint").and_then(Value::as_str) {
            let source_id = source["id"].as_str().unwrap_or_default();
            if source_fingerprint(catalog, source_id).as_deref() != Ok(actual) {
                errors.push(format!("/sources/{index}/fingerprint: does not match the canonical source snapshot"));
            }
        }
    }
    if let Some(actual) = catalog.get("snapshotId").and_then(Value::as_str) {
        if catalog_snapshot_id(catalog).as_deref() != Ok(actual) {
            errors.push("/snapshotId: does not match the canonical catalog snapshot".into());
        }
    }
    for (symbol_index, symbol) in symbols.iter().enumerate() {
        if let Some(evidence) = symbol["evidence"].as_array() {
            for (evidence_index, item) in evidence.iter().enumerate() {
                if let Some(revision) = item.get("revision").and_then(Value::as_str) {
                    let source_id = item["sourceId"].as_str().unwrap_or_default();
                    let expected = sources.iter().find(|source| source["id"].as_str() == Some(source_id)).and_then(|source| source["revision"].as_str());
                    if expected != Some(revision) {
                        errors.push(format!("/symbols/{symbol_index}/evidence/{evidence_index}/revision: does not match referenced source revision"));
                    }
                }
            }
        }
    }
    validate_coverage_consistency(catalog, &source_kinds, errors);
    validate_completeness(catalog, sources, errors);
    reject_secret_fields(catalog, "", errors);
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

    reject_secret_fields(manifest, "/manifest", &mut errors);

    if errors.is_empty() {
        Ok(())
    } else {
        Err(ValidationErrors(errors))
    }
}

fn validate_completeness(catalog: &Value, sources: &[Value], errors: &mut Vec<String>) {
    let Some(completeness) = catalog.get("completeness") else { return; };
    let status = completeness["status"].as_str().unwrap_or_default();
    let mut has_partial_source = false;
    for (index, source) in sources.iter().enumerate() {
        if let Some(collection) = source.get("collection") {
            let state = collection["status"].as_str().unwrap_or_default();
            let truncated = collection["truncated"].as_bool().unwrap_or(false);
            if (state == "succeeded-with-truncation") != truncated {
                errors.push(format!("/sources/{index}/collection: truncation flag must agree with collection status"));
            }
            if state != "succeeded" { has_partial_source = true; }
        }
    }
    if status == "complete" && (has_partial_source || sources.iter().any(|source| source.get("collection").is_none())) {
        errors.push("/completeness/status: complete contradicts one or more non-succeeded source outcomes".into());
    }
    if status == "partial" && !has_partial_source {
        errors.push("/completeness/status: partial requires at least one explicit non-succeeded source outcome".into());
    }
}

fn evidence_compatible(evidence_kind: &str, source_kind: &str) -> bool {
    match evidence_kind {
        "controller-observed" => matches!(
            source_kind,
            "jenkins-controller" | "plugin" | "jenkins-core"
        ),
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
        if matches!(status, "inspected" | "succeeded" | "succeeded-with-truncation") {
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
            let evidence_source_matches = symbol["evidence"].as_array().is_some_and(|evidence| {
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

fn fragment_has_secrets(fragment: &str) -> bool {
    fragment.split('&').any(|part| {
        let key = part.split('=').next().unwrap_or_default();
        is_secret_query_parameter(key)
    })
}

fn url_has_credentials(value: &str) -> bool {
    let authority_has_credentials = value.split_once("://").is_some_and(|(_, remainder)| {
        remainder
            .split('/')
            .next()
            .unwrap_or_default()
            .contains('@')
    });
    let query_has_secrets = value.split_once('?').is_some_and(|(_, query)| {
        query.split('&').any(|part| {
            let key = part.split('=').next().unwrap_or_default();
            is_secret_query_parameter(key)
        })
    });
    let fragment_has_secrets = value
        .split_once('#')
        .is_some_and(|(_, fragment)| fragment_has_secrets(fragment));
    authority_has_credentials || query_has_secrets || fragment_has_secrets
}

fn redact_url_value(value: &str) -> String {
    if let Ok(mut parsed) = url::Url::parse(value) {
        let _ = parsed.set_username("");
        let _ = parsed.set_password(None);
        parsed.set_query(None);
        if parsed.fragment().is_some_and(fragment_has_secrets) {
            parsed.set_fragment(None);
        }
        return parsed.to_string();
    }

    let without_query = value.split_once('?').map_or(value, |(before, _)| before);
    let without_suffix = without_query
        .split_once('#')
        .filter(|(_, fragment)| fragment_has_secrets(fragment))
        .map_or(without_query, |(before, _)| before);
    if let Some((authority, path)) = without_suffix.split_once(':') {
        if authority.contains('@') && !authority.contains('/') {
            let host = authority
                .rsplit_once('@')
                .map_or(authority, |(_, host)| host);
            return format!("{host}:{path}");
        }
    }
    without_suffix.to_owned()
}

fn normalized_secret_key(key: &str) -> String {
    key.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

fn is_secret_query_parameter(key: &str) -> bool {
    let normalized = normalized_secret_key(key);
    is_secret_field_name(key)
        || matches!(
            normalized.as_str(),
            "key" | "auth" | "sig" | "signature" | "xamzsignature" | "xgoogsignature"
        )
}

/// Shared key policy used by catalog validation and ingestion-time redaction.
pub fn is_secret_field_name(key: &str) -> bool {
    let normalized = normalized_secret_key(key);
    [
        "password",
        "passwd",
        "passphrase",
        "pwd",
        "token",
        "secret",
        "credential",
        "authorization",
        "cookie",
        "privatekey",
        "accesskey",
        "session",
        "sessionid",
        "sessiontoken",
        "apikey",
        "authheader",
        "authkey",
        "authmaterial",
        "basicauth",
        "bearer",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

/// Removes secret-bearing fields recursively, strips URL queries, and removes secret-bearing fragments.
/// Safe metadata and non-sensitive URL origins are retained. Returns the number of changes.
pub fn redact_sensitive_fields(value: &mut Value) -> usize {
    match value {
        Value::Object(fields) => {
            let secret_keys: Vec<_> = fields
                .keys()
                .filter(|key| is_secret_field_name(key))
                .cloned()
                .collect();
            let mut redacted = 0;
            for key in secret_keys {
                fields.remove(&key);
                redacted += 1;
            }
            for (key, child) in fields.iter_mut() {
                let normalized = normalized_secret_key(key);
                let is_url = normalized.ends_with("endpoint")
                    || normalized.ends_with("url")
                    || normalized.ends_with("uri");
                if is_url {
                    if let Value::String(url) = child {
                        let safe = redact_url_value(url);
                        if *url != safe {
                            *url = safe;
                            redacted += 1;
                        }
                        continue;
                    }
                }
                redacted += redact_sensitive_fields(child);
            }
            redacted
        }
        Value::Array(items) => items.iter_mut().map(redact_sensitive_fields).sum(),
        _ => 0,
    }
}

fn reject_secret_fields(value: &Value, path: &str, errors: &mut Vec<String>) {
    match value {
        Value::Object(fields) => {
            for (key, child) in fields {
                let child_path = format!("{path}/{key}");
                if is_secret_field_name(key) {
                    errors.push(format!(
                        "{child_path}: secret-bearing fields are not permitted in catalogs"
                    ));
                } else {
                    let normalized = normalized_secret_key(key);
                    let is_url = normalized.ends_with("endpoint")
                        || normalized.ends_with("url")
                        || normalized.ends_with("uri");
                    if is_url && child.as_str().is_some_and(url_has_credentials) {
                        errors.push(format!(
                            "{child_path}: URL contains embedded credentials or secret query parameters or fragments"
                        ));
                    }
                    reject_secret_fields(child, &child_path, errors);
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
        let error = validate_catalog(&invalid, &schema())
            .unwrap_err()
            .to_string();
        assert!(error.contains("/symbols/0/fieldEvidence/examples"));
        assert!(error.contains("absent from this symbol"));
    }

    #[test]
    fn enforces_evidence_kind_and_source_kind_compatibility() {
        let mut valid = valid_catalog();
        valid["symbols"][0]["evidence"][0]["sourceId"] = json!("platform-lib");
        valid["symbols"][0]["evidence"][0]["kind"] = json!("source-scanned");
        assert!(validate_catalog(&valid, &schema()).is_ok());

        let mut invalid = valid_catalog();
        invalid["symbols"][0]["evidence"][0]["sourceId"] = json!("platform-lib");
        let error = validate_catalog(&invalid, &schema())
            .unwrap_err()
            .to_string();
        assert!(error.contains("/symbols/0/evidence/0/kind"));
        assert!(error.contains("incompatible with source 'platform-lib' of kind 'shared-library'"));
    }

    #[test]
    fn rejects_coverage_claims_contradicted_by_catalog_records() {
        let mut invalid = valid_catalog();
        invalid["coverage"] = json!([{
            "sourceId":"controller",
            "surface":"pipeline-steps",
            "status":"not-collected"
        }]);
        let error = validate_catalog(&invalid, &schema())
            .unwrap_err()
            .to_string();
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
        let error = validate_catalog(&catalog, &schema())
            .unwrap_err()
            .to_string();
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
    #[test]
    fn detects_normalized_secret_key_variants_without_flagging_safe_metadata() {
        for key in [
            "apiKey",
            "API_KEY",
            "x-api-key",
            "Authorization",
            "clientSecret",
            "credentialId",
            "privateKey",
            "refreshToken",
            "set-cookie",
            "passwordHash",
            "AWS_SECRET_ACCESS_KEY",
            "sessionId",
            "authHeader",
            "bearerToken",
        ] {
            assert!(is_secret_field_name(key), "{key}");
        }
        for key in [
            "description",
            "displayName",
            "username",
            "apiBaseUrl",
            "repositoryUrl",
            "key",
            "signature",
        ] {
            assert!(!is_secret_field_name(key), "{key}");
        }
        assert!(is_secret_query_parameter("signature"));
        assert!(is_secret_query_parameter("X-Amz-Signature"));
    }

    #[test]
    fn recursively_redacts_secret_fields_and_url_request_modifiers() {
        let mut value = json!({
            "metadata": {
                "safeLabel": "preserved",
                "apiKey": "top-secret-api",
                "nested": [
                    {"refreshToken": "top-secret-refresh", "description": "safe"},
                    {"credentialId": "jenkins-credential-123"}
                ]
            },
            "headers": {
                "Authorization": "Bearer top-secret-auth",
                "Content-Type": "application/json"
            },
            "documentationUrl": "https://docs-user:top-secret-url@docs.example.test/step?access_token=top-secret#details",
            "apiEndpoint": "https://user:top-secret-endpoint@controller.example.test/api?token=top-secret-query#details",
            "privateDocumentationUrl": "https://docs.example.test/page#access_token=top-secret-fragment",
            "repositoryUrl": "git@github.example.test:team/library.git"
        });
        let changes = redact_sensitive_fields(&mut value);
        let serialized = value.to_string();
        assert!(changes >= 4);
        assert!(!serialized.contains("top-secret"));
        assert!(!serialized.contains("credential-123"));
        assert!(!serialized.contains("apiKey"));
        assert!(!serialized.contains("Authorization"));
        assert_eq!(value["metadata"]["safeLabel"], "preserved");
        assert_eq!(value["metadata"]["nested"][0]["description"], "safe");
        assert_eq!(value["headers"]["Content-Type"], "application/json");
        assert_eq!(
            value["documentationUrl"],
            "https://docs.example.test/step#details"
        );
        assert_eq!(
            value["apiEndpoint"],
            "https://controller.example.test/api#details"
        );
        assert_eq!(
            value["privateDocumentationUrl"],
            "https://docs.example.test/page"
        );
        assert_eq!(
            value["repositoryUrl"],
            "github.example.test:team/library.git"
        );
    }

    #[test]
    fn rejects_secret_fields_in_nested_extensible_metadata_and_secret_url_queries() {
        let mut invalid = valid_catalog();
        invalid["sources"][0]["metadata"] = json!({
            "safe": "preserved",
            "nested": [{"credentialId": "do-not-echo"}]
        });
        let errors = validate_catalog(&invalid, &schema())
            .unwrap_err()
            .to_string();
        assert!(errors.contains("/sources/0/metadata/nested/0/credentialId"));
        assert!(!errors.contains("do-not-echo"));

        let mut invalid = valid_catalog();
        invalid["sources"][0]["url"] =
            json!("https://git.example.test/team/repo?X-Api-Key=do-not-echo");
        let errors = validate_catalog(&invalid, &schema())
            .unwrap_err()
            .to_string();
        assert!(errors.contains("secret query parameters"));
        assert!(!errors.contains("do-not-echo"));

        let mut invalid = valid_catalog();
        invalid["sources"][0]["metadata"] = json!({
            "apiEndpoint": "https://user:password@controller.example.test/api"
        });
        let errors = validate_catalog(&invalid, &schema())
            .unwrap_err()
            .to_string();
        assert!(errors.contains("/sources/0/metadata/apiEndpoint"));
        assert!(!errors.contains("password"));

        let mut invalid = valid_catalog();
        invalid["sources"][0]["metadata"] = json!({
            "documentationUrl": "https://docs.example.test/page#access_token=do-not-echo"
        });
        let errors = validate_catalog(&invalid, &schema())
            .unwrap_err()
            .to_string();
        assert!(errors.contains("/sources/0/metadata/documentationUrl"));
        assert!(errors.contains("secret query parameters or fragments"));
        assert!(!errors.contains("do-not-echo"));
    }

    #[test]
    fn rejects_secret_fields_in_forward_compatible_manifest_metadata() {
        let catalog = valid_catalog();
        let mut manifest = manifest_for(&catalog);
        manifest["provenance"]["platform-lib"]["futureMetadata"] =
            json!({"nested": [{"accessToken": "do-not-echo"}]});
        let error = validate_package_metadata(&catalog, &manifest)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("/manifest/provenance/platform-lib/futureMetadata/nested/0/accessToken")
        );
        assert!(!error.contains("do-not-echo"));
    }
    #[test]
    fn canonical_snapshot_identity_ignores_collection_timestamps_and_array_order() {
        let mut first = valid_catalog();
        first["generatedAt"] = json!("2026-11-26T10:00:00Z");
        first["sources"][0]["metadata"] = json!({"retrievedAt":"2026-11-26T10:00:00Z","stable":"value"});
        stamp_catalog_identity(&mut first).unwrap();
        let first_id = first["snapshotId"].as_str().unwrap().to_owned();

        let mut second = valid_catalog();
        second["generatedAt"] = json!("2026-11-27T10:00:00Z");
        second["sources"][0]["metadata"] = json!({"stable":"value","retrievedAt":"2026-11-27T10:00:00Z"});
        second["symbols"].as_array_mut().unwrap().reverse();
        stamp_catalog_identity(&mut second).unwrap();

        assert_eq!(first_id, second["snapshotId"].as_str().unwrap());
        assert_eq!(first["sources"][0]["fingerprint"], second["sources"][0]["fingerprint"]);
        assert!(validate_catalog(&first, &schema()).is_ok());
        assert!(validate_catalog(&second, &schema()).is_ok());
    }

    #[test]
    fn rejects_source_or_catalog_fingerprint_drift() {
        let mut catalog = valid_catalog();
        stamp_catalog_identity(&mut catalog).unwrap();
        catalog["symbols"][0]["description"] = json!("changed after fingerprinting");
        let error = validate_catalog(&catalog, &schema()).unwrap_err().to_string();
        assert!(error.contains("/sources/0/fingerprint"));
        assert!(error.contains("/snapshotId"));
    }

    #[test]
    fn validates_source_collection_completeness_and_evidence_revision() {
        let mut catalog = valid_catalog();
        catalog["sources"][0]["collection"] = json!({
            "profile":"deep",
            "status":"succeeded",
            "truncated":false
        });
        catalog["completeness"] = json!({"status":"complete"});
        catalog["symbols"][0]["evidence"][0]["revision"] = catalog["sources"][0]["revision"].clone();
        stamp_catalog_identity(&mut catalog).unwrap();
        assert!(validate_catalog(&catalog, &schema()).is_ok());

        catalog["sources"][0]["collection"]["status"] = json!("succeeded-with-truncation");
        let error = validate_catalog(&catalog, &schema()).unwrap_err().to_string();
        assert!(error.contains("truncation flag must agree"));

        catalog["sources"][0]["collection"]["status"] = json!("succeeded");
        catalog["sources"][0]["collection"]["truncated"] = json!(false);
        catalog["completeness"]["status"] = json!("partial");
        let error = validate_catalog(&catalog, &schema()).unwrap_err().to_string();
        assert!(error.contains("partial requires at least one explicit non-succeeded source outcome"));
    }

    #[test]
    fn rejects_evidence_revision_that_does_not_match_its_source() {
        let mut catalog = valid_catalog();
        catalog["symbols"][0]["evidence"][0]["revision"] = json!("stale-revision");
        let error = validate_catalog(&catalog, &schema()).unwrap_err().to_string();
        assert!(error.contains("does not match referenced source revision"));
    }

}
