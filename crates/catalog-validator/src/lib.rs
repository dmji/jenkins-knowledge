use jsonschema::validator_for;
use serde_json::Value;
use std::collections::HashSet;
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
        validate_semantics(catalog, &mut errors);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(ValidationErrors(errors))
    }
}

fn validate_semantics(catalog: &Value, errors: &mut Vec<String>) {
    let sources = catalog["sources"].as_array().expect("schema validated sources");
    let symbols = catalog["symbols"].as_array().expect("schema validated symbols");
    let mut source_ids = HashSet::new();
    let mut symbol_ids = HashSet::new();

    for (index, source) in sources.iter().enumerate() {
        let id = source["id"].as_str().expect("schema validated source id");
        if !source_ids.insert(id) {
            errors.push(format!("/sources/{index}/id: duplicate source id '{id}'"));
        }
        if let Some(url) = source["url"].as_str() {
            if url_has_credentials(url) {
                errors.push(format!("/sources/{index}/url: URLs must not contain credentials or query parameters"));
            }
        }
        reject_secret_fields(source, &format!("/sources/{index}"), errors);
    }

    if let Some(coverage) = catalog["coverage"].as_array() {
        for (index, entry) in coverage.iter().enumerate() {
            let source_id = entry["sourceId"].as_str().expect("schema validated coverage source id");
            if !source_ids.contains(source_id) {
                errors.push(format!("/coverage/{index}/sourceId: unknown source id '{source_id}'"));
            }
        }
    }

    for (index, symbol) in symbols.iter().enumerate() {
        let id = symbol["id"].as_str().expect("schema validated symbol id");
        if !symbol_ids.insert(id) {
            errors.push(format!("/symbols/{index}/id: duplicate symbol id '{id}'"));
        }
        let source_id = symbol["sourceId"].as_str().expect("schema validated source id");
        if !source_ids.contains(source_id) {
            errors.push(format!("/symbols/{index}/sourceId: unknown source id '{source_id}'"));
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
        reject_secret_fields(symbol, &format!("/symbols/{index}"), errors);
    }
}

fn url_has_credentials(value: &str) -> bool {
    let Some((_, remainder)) = value.split_once("://") else {
        return false;
    };
    let authority = remainder.split('/').next().unwrap_or_default();
    authority.contains('@') || value.split_once('?').is_some()
}

fn reject_secret_fields(value: &Value, path: &str, errors: &mut Vec<String>) {
    match value {
        Value::Object(fields) => {
            for (key, child) in fields {
                let normalized = key.to_ascii_lowercase().replace(['_', '-'], "");
                if ["password", "token", "apitoken", "authorization", "secret", "credentials", "clientsecret"]
                    .contains(&normalized.as_str())
                {
                    errors.push(format!("{path}/{key}: secret-bearing fields are not permitted in catalogs"));
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
        assert!(validate_catalog(&invalid, &schema()).unwrap_err().to_string().contains("unknown source id"));
    }

    #[test]
    fn rejects_duplicate_symbol_ids() {
        let mut invalid = valid_catalog();
        let duplicate = invalid["symbols"][0].clone();
        invalid["symbols"].as_array_mut().unwrap().push(duplicate);
        assert!(validate_catalog(&invalid, &schema()).unwrap_err().to_string().contains("duplicate symbol id"));
    }

    #[test]
    fn rejects_duplicate_source_ids() {
        let mut invalid = valid_catalog();
        let duplicate = invalid["sources"][0].clone();
        invalid["sources"].as_array_mut().unwrap().push(duplicate);
        assert!(validate_catalog(&invalid, &schema()).unwrap_err().to_string().contains("duplicate source id"));
    }

    #[test]
    fn rejects_duplicate_parameter_names() {
        let mut invalid = valid_catalog();
        invalid["symbols"][0]["parameters"] = json!([{"name":"script"},{"name":"script"}]);
        assert!(validate_catalog(&invalid, &schema()).unwrap_err().to_string().contains("duplicate parameter name"));
    }

    #[test]
    fn rejects_secret_fields_and_credential_urls() {
        let mut invalid = valid_catalog();
        invalid["sources"][0]["metadata"] = json!({"api_token":"must-not-leak"});
        let errors = validate_catalog(&invalid, &schema()).unwrap_err().to_string();
        assert!(errors.contains("secret-bearing fields"));
        assert!(!errors.contains("must-not-leak"));
    }

    #[test]
    fn rejects_urls_with_embedded_credentials_or_query_parameters() {
        let mut invalid = valid_catalog();
        invalid["sources"][0]["url"] = json!("https://user:password@example.test/repo");
        assert!(validate_catalog(&invalid, &schema()).unwrap_err().to_string().contains("must not contain credentials"));
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
}
