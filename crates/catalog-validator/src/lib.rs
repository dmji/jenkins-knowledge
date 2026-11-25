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
        validate_references(catalog, &mut errors);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(ValidationErrors(errors))
    }
}

fn validate_references(catalog: &Value, errors: &mut Vec<String>) {
    let Some(sources) = catalog.get("sources").and_then(Value::as_array) else {
        return;
    };
    let Some(symbols) = catalog.get("symbols").and_then(Value::as_array) else {
        return;
    };

    let mut source_ids = HashSet::new();
    for (index, source) in sources.iter().enumerate() {
        if let Some(id) = source.get("id").and_then(Value::as_str) {
            if !source_ids.insert(id) {
                errors.push(format!("/sources/{index}/id: duplicate source id '{id}'"));
            }
        }
    }

    let mut symbol_ids = HashSet::new();
    for (index, symbol) in symbols.iter().enumerate() {
        if let Some(id) = symbol.get("id").and_then(Value::as_str) {
            if !symbol_ids.insert(id) {
                errors.push(format!("/symbols/{index}/id: duplicate symbol id '{id}'"));
            }
        }
        if let Some(source_id) = symbol.get("sourceId").and_then(Value::as_str) {
            if !source_ids.contains(source_id) {
                errors.push(format!(
                    "/symbols/{index}/sourceId: unknown source id '{source_id}'"
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Value {
        serde_json::from_str(include_str!("../../../schemas/catalog.schema.json")).unwrap()
    }

    #[test]
    fn accepts_valid_catalog() {
        let catalog: Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/catalog-valid.json"))
                .unwrap();
        assert!(validate_catalog(&catalog, &schema()).is_ok());
    }

    #[test]
    fn rejects_missing_required_fields() {
        let catalog = json!({"schemaVersion": "1.0.0"});
        let errors = validate_catalog(&catalog, &schema()).unwrap_err();
        assert!(errors.to_string().contains("catalogId"));
    }

    #[test]
    fn rejects_unknown_symbol_source() {
        let catalog: Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/catalog-valid.json"))
                .unwrap();
        let mut invalid = catalog;
        invalid["symbols"][0]["sourceId"] = json!("missing-source");
        let errors = validate_catalog(&invalid, &schema()).unwrap_err();
        assert!(errors.to_string().contains("unknown source id"));
    }

    #[test]
    fn rejects_duplicate_symbol_ids() {
        let catalog: Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/catalog-valid.json"))
                .unwrap();
        let mut invalid = catalog;
        let duplicate = invalid["symbols"][0].clone();
        invalid["symbols"].as_array_mut().unwrap().push(duplicate);
        let errors = validate_catalog(&invalid, &schema()).unwrap_err();
        assert!(errors.to_string().contains("duplicate symbol id"));
    }
}
