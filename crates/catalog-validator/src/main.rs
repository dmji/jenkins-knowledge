use catalog_validator::validate_catalog;
use serde_json::Value;
use std::{env, fs, process};

fn main() {
    let mut args = env::args().skip(1);
    let catalog_path = args.next().unwrap_or_else(|| {
        eprintln!("usage: catalog-validator <catalog.json> <schema.json>");
        process::exit(2);
    });
    let schema_path = args.next().unwrap_or_else(|| {
        eprintln!("usage: catalog-validator <catalog.json> <schema.json>");
        process::exit(2);
    });

    let catalog = read_json(&catalog_path);
    let schema = read_json(&schema_path);

    match validate_catalog(&catalog, &schema) {
        Ok(()) => println!("valid: {catalog_path}"),
        Err(errors) => {
            eprintln!("invalid: {catalog_path}\n{errors}");
            process::exit(1);
        }
    }
}

fn read_json(path: &str) -> Value {
    let contents = fs::read_to_string(path).unwrap_or_else(|error| {
        eprintln!("cannot read {path}: {error}");
        process::exit(2);
    });
    serde_json::from_str(&contents).unwrap_or_else(|error| {
        eprintln!("invalid JSON in {path}: {error}");
        process::exit(2);
    })
}
