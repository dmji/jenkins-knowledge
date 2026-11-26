use jenkins_runtime::{Runtime, SearchOptions, MAX_SEARCH_RESULTS};
use lsp_server::{Connection, Message, Notification, Request, Response};
use serde_json::{json, Value};
use std::{collections::HashMap, error::Error, process};

fn log(connection: &Connection, message: impl Into<String>, kind: i64) {
    let notification = Notification::new(
        "window/logMessage".into(),
        json!({"type": kind, "message": message.into()}),
    );
    let _ = connection.sender.send(Message::Notification(notification));
}

fn open_runtime(settings: &Value) -> Result<Runtime, Box<dyn Error>> {
    let package = settings.get("packagePath").and_then(Value::as_str)
        .ok_or("set jenkinsKnowledge.packagePath to a Knowledge Package directory")?;
    let runtime = Runtime::open(package)?;
    if let Some(profile) = settings.get("profile").and_then(Value::as_str) {
        if !profile.is_empty() && profile != runtime.profile() {
            return Err(format!("configured profile '{profile}' does not match package profile '{}'; select the matching package", runtime.profile()).into());
        }
    }
    Ok(runtime)
}

fn settings(params: &Value) -> &Value {
    let settings = params.get("settings").unwrap_or(params);
    settings.get("jenkinsKnowledge").unwrap_or(settings)
}

fn word_at(text: &str, line_number: usize, utf16_column: usize) -> Option<String> {
    let line = text.lines().nth(line_number)?;
    let mut byte_column = 0;
    let mut units = 0;
    for (index, character) in line.char_indices() {
        if units >= utf16_column { break; }
        units += character.len_utf16();
        byte_column = index + character.len_utf8();
    }
    if units < utf16_column { byte_column = line.len(); }

    let mut start = byte_column;
    while start > 0 {
        let previous = line[..start].char_indices().next_back()?;
        if !(previous.1.is_alphanumeric() || matches!(previous.1, '_' | '$')) { break; }
        start = previous.0;
    }
    let mut end = byte_column;
    while end < line.len() {
        let character = line[end..].chars().next()?;
        if !(character.is_alphanumeric() || matches!(character, '_' | '$')) { break; }
        end += character.len_utf8();
    }
    (start < end).then(|| line[start..end].to_string())
}

fn prefix_at(text: &str, line_number: usize, utf16_column: usize) -> String {
    let Some(line) = text.lines().nth(line_number) else { return String::new(); };
    let mut byte_column = 0;
    let mut units = 0;
    for (index, character) in line.char_indices() {
        if units + character.len_utf16() > utf16_column { break; }
        units += character.len_utf16();
        byte_column = index + character.len_utf8();
    }
    let mut start = byte_column;
    while start > 0 {
        let Some((index, character)) = line[..start].char_indices().next_back() else { break; };
        if !(character.is_alphanumeric() || matches!(character, '_' | '$')) { break; }
        start = index;
    }
    line[start..byte_column].to_owned()
}

fn block_is_open(before: &str, block: &str) -> bool {
    let needle = format!("{block} {{");
    before.rfind(&needle).is_some_and(|open| {
        before.rfind('}').is_none_or(|close| open > close)
    })
}

fn completion_context(text: &str, line: usize, character: usize) -> (String, Vec<String>, Vec<String>) {
    let prefix = prefix_at(text, line, character);
    let lines: Vec<_> = text.lines().take(line).collect();
    let mut before = lines.join("\n");
    if let Some(current) = text.lines().nth(line) {
        if !before.is_empty() { before.push('\n'); }
        let byte_column = current.char_indices()
            .scan(0usize, |units, (index, ch)| {
                let current_units = *units;
                *units += ch.len_utf16();
                Some((index, ch, current_units))
            })
            .take_while(|(_, ch, units)| *units + ch.len_utf16() <= character)
            .map(|(index, ch, _)| index + ch.len_utf8())
            .last().unwrap_or(0);
        before.push_str(&current[..byte_column]);
    }

    let in_script = block_is_open(&before, "script");
    let in_steps = block_is_open(&before, "steps");
    let in_stage = block_is_open(&before, "stage");
    let declarative = before.contains("pipeline {") && !in_script;
    let mut contexts = vec!["pipeline".to_owned()];
    if in_steps { contexts.push("steps".into()); }
    if in_stage { contexts.push("stage".into()); }
    let kinds = if declarative && !in_steps {
        vec!["step".into(), "declarative-directive".into()]
    } else {
        vec!["step".into()]
    };
    (prefix, contexts, kinds)
}

fn supported_symbol(symbol: &Value) -> bool {
    symbol["kind"] == "step"
        || symbol["kind"] == "declarative-directive"
        || symbol["source"]["kind"] == "shared-library"
}

fn completion(runtime: Option<&Runtime>, text: &str, line: usize, character: usize) -> Value {
    let Some(runtime) = runtime else { return json!({"isIncomplete": false, "items": []}); };
    let (prefix, contexts, kinds) = completion_context(text, line, character);
    let common = SearchOptions {
        prefix: Some(prefix),
        contexts: Some(contexts),
        limit: MAX_SEARCH_RESULTS,
        ..SearchOptions::default()
    };
    let primary = SearchOptions { kinds: Some(kinds), ..common.clone() };
    let library = SearchOptions {
        kinds: Some(vec!["global-variable".into(), "method".into(), "class".into(), "property".into()]),
        source_kind: Some("shared-library".into()),
        ..common
    };
    let primary = runtime.search_page(&primary, 0).expect("zero offset is valid");
    let library = runtime.search_page(&library, 0).expect("zero offset is valid");
    let total = primary.total + library.total;
    let mut symbols = primary.results;
    symbols.extend(library.results);
    symbols.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str())
        .then_with(|| left["id"].as_str().cmp(&right["id"].as_str())));
    symbols.dedup_by(|left, right| left["id"] == right["id"]);
    symbols.truncate(MAX_SEARCH_RESULTS);

    let items: Vec<Value> = symbols.iter().filter(|symbol| supported_symbol(symbol)).map(|symbol| {
        let name = symbol["name"].as_str().unwrap_or_default();
        let kind = symbol["kind"].as_str().unwrap_or_default();
        let lsp_kind = match kind {
            "step" => 3,
            "declarative-directive" => 14,
            "global-variable" => 6,
            "class" => 7,
            "property" => 10,
            _ => 2,
        };
        let signature = symbol["signature"].as_str().unwrap_or_default();
        let description = symbol["description"].as_str().unwrap_or_default();
        let source = symbol["source"]["name"].as_str()
            .or_else(|| symbol["sourceId"].as_str()).unwrap_or("unknown source");
        let confidence = symbol["confidence"].as_str().unwrap_or("unknown");
        json!({
            "label": name,
            "kind": lsp_kind,
            "detail": if signature.is_empty() { format!("{kind} · {source}") } else { format!("{signature} · {source}") },
            "documentation": {"kind": "markdown", "value": format!("{}\n\n**Source:** {}\n\n**Confidence:** {}\n\n**Profile:** {}", description, source, confidence, runtime.profile())},
            "insertText": name,
            "filterText": name,
            "sortText": format!("{kind}-{name}"),
            "data": {"id": symbol["id"], "profile": runtime.profile()}
        })
    }).collect();
    json!({"isIncomplete": total > items.len(), "items": items})
}

fn hover(runtime: Option<&Runtime>, word: Option<String>) -> Value {
    let (Some(runtime), Some(word)) = (runtime, word) else { return Value::Null; };
    let options = SearchOptions { query: word.clone(), limit: MAX_SEARCH_RESULTS, ..SearchOptions::default() };
    let symbols: Vec<_> = runtime.search_with_options(&options).into_iter()
        .filter(|symbol| supported_symbol(symbol) && symbol["name"].as_str() == Some(word.as_str())).collect();
    if symbols.is_empty() { return Value::Null; }
    let sections = symbols.iter().map(|symbol| {
        let name = symbol["name"].as_str().unwrap_or_default();
        let signature = symbol["signature"].as_str().unwrap_or("Signature unavailable");
        let description = symbol["description"].as_str().unwrap_or("No description available.");
        let confidence = symbol["confidence"].as_str().unwrap_or("unknown");
        let source = symbol["source"]["name"].as_str()
            .or_else(|| symbol["sourceId"].as_str()).unwrap_or("unknown source");
        format!("**{name}**\n\nGroovy: {signature}\n\n{description}\n\n**Source:** {source}  \n**Confidence:** {confidence}")
    }).collect::<Vec<_>>().join("\n\n---\n\n");
    json!({"contents": {"kind": "markdown", "value": format!("{sections}\n\n**Profile:** {}", runtime.profile())}})
}

fn run(connection: Connection, initialization: Value) -> Result<(), Box<dyn Error>> {
    let mut runtime = match open_runtime(initialization.get("initializationOptions").unwrap_or(&Value::Null)) {
        Ok(runtime) => Some(runtime),
        Err(error) => { log(&connection, error.to_string(), 1); None }
    };
    let mut documents: HashMap<String, String> = HashMap::new();

    while let Ok(message) = connection.receiver.recv() {
        match message {
            Message::Request(request) => handle_request(&connection, request, runtime.as_ref(), &documents)?,
            Message::Notification(notification) => match notification.method.as_str() {
                "exit" => break,
                "textDocument/didOpen" => {
                    if let (Some(uri), Some(text)) = (
                        notification.params.pointer("/textDocument/uri").and_then(Value::as_str),
                        notification.params.pointer("/textDocument/text").and_then(Value::as_str),
                    ) { documents.insert(uri.into(), text.into()); }
                }
                "textDocument/didChange" => {
                    if let (Some(uri), Some(text)) = (
                        notification.params.pointer("/textDocument/uri").and_then(Value::as_str),
                        notification.params.pointer("/contentChanges/0/text").and_then(Value::as_str),
                    ) { documents.insert(uri.into(), text.into()); }
                }
                "textDocument/didClose" => {
                    if let Some(uri) = notification.params.pointer("/textDocument/uri").and_then(Value::as_str) {
                        documents.remove(uri);
                    }
                }
                "workspace/didChangeConfiguration" => {
                    runtime = match open_runtime(settings(&notification.params)) {
                        Ok(runtime) => {
                            log(&connection, format!("Loaded Knowledge Package profile '{}'", runtime.profile()), 3);
                            Some(runtime)
                        }
                        Err(error) => { log(&connection, error.to_string(), 1); None }
                    };
                }
                _ => {}
            },
            Message::Response(_) => {}
        }
    }
    Ok(())
}

fn handle_request(connection: &Connection, request: Request, runtime: Option<&Runtime>,
    documents: &HashMap<String, String>) -> Result<(), Box<dyn Error>> {
    let result = match request.method.as_str() {
        "shutdown" => Value::Null,
        "textDocument/completion" => {
            let uri = request.params.pointer("/textDocument/uri").and_then(Value::as_str).unwrap_or_default();
            let line = request.params.pointer("/position/line").and_then(Value::as_u64).unwrap_or(0) as usize;
            let character = request.params.pointer("/position/character").and_then(Value::as_u64).unwrap_or(0) as usize;
            let text = documents.get(uri).map(String::as_str).unwrap_or("");
            completion(runtime, text, line, character)
        },
        "jenkinsKnowledge/coverage" => json!({
            "profile": runtime.map(Runtime::profile),
            "coverage": runtime.map(Runtime::coverage).unwrap_or(&[])
        }),
        "jenkinsKnowledge/search" => {
            let Some(runtime) = runtime else {
                connection.sender.send(Message::Response(Response::new_err(
                    request.id, lsp_server::ErrorCode::InvalidRequest as i32,
                    "No Knowledge Package is loaded".to_string()
                )))?;
                return Ok(());
            };
            let limit = request.params.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
            if !(1..=MAX_SEARCH_RESULTS).contains(&limit) {
                connection.sender.send(Message::Response(Response::new_err(
                    request.id, lsp_server::ErrorCode::InvalidParams as i32,
                    format!("limit must be between 1 and {MAX_SEARCH_RESULTS}")
                )))?;
                return Ok(());
            }
            let options = SearchOptions {
                query: request.params.get("query").and_then(Value::as_str).unwrap_or("").to_owned(),
                kind: request.params.get("kind").and_then(Value::as_str).map(str::to_owned),
                source: request.params.get("source").and_then(Value::as_str).map(str::to_owned),
                plugin: request.params.get("plugin").and_then(Value::as_str).map(str::to_owned),
                owner: request.params.get("owner").and_then(Value::as_str).map(str::to_owned),
                type_name: request.params.get("type").and_then(Value::as_str).map(str::to_owned),
                confidence: request.params.get("confidence").and_then(Value::as_str).map(str::to_owned),
                prefix: None,
                contexts: None,
                kinds: None,
                source_kind: None,
                limit,
            };
            json!({"profile": runtime.profile(), "limit": limit, "results": runtime.search_with_options(&options)})
        },
        "textDocument/hover" => {
            let uri = request.params.pointer("/textDocument/uri").and_then(Value::as_str).unwrap_or_default();
            let line = request.params.pointer("/position/line").and_then(Value::as_u64).unwrap_or(0) as usize;
            let character = request.params.pointer("/position/character").and_then(Value::as_u64).unwrap_or(0) as usize;
            let word = documents.get(uri).and_then(|text| word_at(text, line, character));
            hover(runtime, word)
        }
        _ => {
            let response = Response::new_err(request.id, lsp_server::ErrorCode::MethodNotFound as i32,
                format!("unsupported method: {}", request.method));
            connection.sender.send(Message::Response(response))?;
            return Ok(());
        }
    };
    connection.sender.send(Message::Response(Response::new_ok(request.id, result)))?;
    Ok(())
}

fn main() {
    if let Err(error) = serve() {
        eprintln!("jenkins-lsp: {error}");
        process::exit(1);
    }
}

fn serve() -> Result<(), Box<dyn Error>> {
    let (connection, io_threads) = Connection::stdio();
    let (initialize_id, initialization) = connection.initialize_start()?;
    connection.initialize_finish(initialize_id, json!({
        "capabilities": {
            "textDocumentSync": {"openClose": true, "change": 1},
            "completionProvider": {"resolveProvider": false, "triggerCharacters": ["."]},
            "hoverProvider": true
        },
        "serverInfo": {"name": "jenkins-knowledge", "version": env!("CARGO_PKG_VERSION")}
    }))?;
    run(connection, initialization)?;
    io_threads.join()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;
    use std::fs;

    fn fixture_runtime(profile: &str) -> Runtime {
        let package = std::env::temp_dir().join(format!("jenkins-lsp-test-{}", process::id()));
        let _ = fs::remove_dir_all(&package);
        fs::create_dir_all(&package).unwrap();
        let catalog = include_str!("../../../tests/fixtures/catalog-valid.json");
        fs::write(package.join("catalog.json"), catalog).unwrap();
        let checksum = sha2::Sha256::digest(catalog.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        fs::write(package.join("manifest.json"), json!({
            "packageVersion": "1.0.0",
            "catalogSchemaVersion": "1.0.0",
            "profile": profile,
            "createdAt": "2026-01-01T00:00:00Z",
            "catalog": "catalog.json",
            "sourceRevisions": {},
            "provenance": {},
            "checksums": {"catalog.json": checksum},
            "toolVersions": {}
        }).to_string()).unwrap();
        let previous = std::env::current_dir().unwrap();
        std::env::set_current_dir(&package).unwrap();
        let runtime = Runtime::open(&package).unwrap();
        std::env::set_current_dir(previous).unwrap();
        fs::remove_dir_all(package).unwrap();
        runtime
    }

    #[test]
    fn finds_identifier_at_cursor_using_utf16_columns() {
        assert_eq!(word_at("écho deployApp()", 0, 10).as_deref(), Some("deployApp"));
    }

    #[test]
    fn ignores_punctuation_and_empty_lines() {
        assert_eq!(word_at("sh 'hello world'", 0, 2).as_deref(), Some("sh"));
        assert_eq!(word_at("sh 'hello world'", 0, 3), None);
        assert_eq!(word_at("first\nsecond", 1, 3).as_deref(), Some("second"));
    }

    #[test]
    fn catalog_scope_includes_steps_and_shared_library_symbols_only() {
        assert!(supported_symbol(&json!({"kind":"step"})));
        assert!(supported_symbol(&json!({"kind":"method","source":{"kind":"shared-library"}})));
        assert!(!supported_symbol(&json!({"kind":"directive","source":{"kind":"jenkins"}})));
    }

    #[test]
    fn completion_and_hover_use_the_selected_catalog() {
        let runtime = fixture_runtime("fixture");
        let items = completion(Some(&runtime), "", 0, 0);
        let labels: Vec<_> = items["items"].as_array().unwrap().iter()
            .filter_map(|item| item["label"].as_str()).collect();
        assert!(labels.contains(&"sh"));
        assert!(labels.contains(&"deployApp"));
        assert!(!labels.contains(&"agent"));

        let details = hover(Some(&runtime), Some("sh".into()));
        let value = details["contents"]["value"].as_str().unwrap();
        assert!(value.contains("sh(Map args)"));
        assert!(value.contains("Runs a shell command."));
        assert!(value.contains("Jenkins fixture"));
        assert!(value.contains("verified"));
        assert!(value.contains("fixture"));
    }
}
