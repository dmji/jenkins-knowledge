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

const MAX_CONTEXT_SCAN_BYTES: usize = 1024 * 1024;

fn relevant_block(name: &str) -> Option<&'static str> {
    match name {
        "pipeline" | "stages" | "stage" | "steps" | "post" | "script" | "agent"
        | "environment" | "options" | "parameters" | "triggers" | "tools" | "when"
        | "parallel" | "matrix" | "axes" | "excludes" | "input" => {
            // The matched token is one of the static literals above.
            Some(match name {
                "pipeline" => "pipeline",
                "stages" => "stages",
                "stage" => "stage",
                "steps" => "steps",
                "post" => "post",
                "script" => "script",
                "agent" => "agent",
                "environment" => "environment",
                "options" => "options",
                "parameters" => "parameters",
                "triggers" => "triggers",
                "tools" => "tools",
                "when" => "when",
                "parallel" => "parallel",
                "matrix" => "matrix",
                "axes" => "axes",
                "excludes" => "excludes",
                "input" => "input",
                _ => unreachable!(),
            })
        }
        _ => None,
    }
}

fn skip_quoted_string(bytes: &[u8], start: usize) -> usize {
    let quote = bytes[start];
    let triple = bytes.get(start..start + 3) == Some(&[quote, quote, quote]);
    let delimiter = if triple { 3 } else { 1 };
    let mut index = start + delimiter;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index = (index + 2).min(bytes.len());
        } else if triple && bytes.get(index..index + 3) == Some(&[quote, quote, quote]) {
            return index + 3;
        } else if !triple && bytes[index] == quote {
            return index + 1;
        } else {
            index += 1;
        }
    }
    bytes.len()
}

fn skip_slashy_string(bytes: &[u8], start: usize) -> usize {
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index = (index + 2).min(bytes.len());
        } else if bytes[index] == b'/' {
            return index + 1;
        } else {
            index += 1;
        }
    }
    bytes.len()
}

fn skip_dollar_slashy_string(bytes: &[u8], start: usize) -> usize {
    let mut index = start + 2;
    while index < bytes.len() {
        if bytes[index] == b'$' && (bytes.get(index + 1) == Some(&b' {
            index += 2;
        } else if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'$') {
            return index + 2;
        } else {
            index += 1;
        }
    }
    bytes.len()
}

fn slashy_string_can_start(bytes: &[u8], index: usize) -> bool {
    let mut previous = index;
    while previous > 0 && bytes[previous - 1].is_ascii_whitespace() {
        previous -= 1;
    }
    if previous == 0 {
        return true;
    }
    if matches!(bytes[previous - 1], b'=' | b'(' | b'[' | b'{' | b',' | b':' | b';'
        | b'!' | b'?' | b'&' | b'|' | b'~' | b'>') {
        return true;
    }
    let end = previous;
    while previous > 0 && (bytes[previous - 1].is_ascii_alphanumeric()
        || matches!(bytes[previous - 1], b'_' | b'$')) {
        previous -= 1;
    }
    let word = &bytes[previous..end];
    word == b"return" || word == b"throw" || word == b"case"
        || word == b"in" || word == b"assert"
}

/// Return the relevant Declarative block stack before the cursor. This is a
/// bounded lexical recognizer, not a Groovy parser: it skips comments and all
/// Groovy string literal forms and only accepts block names at statement starts
/// outside map/list literals and ordinary Groovy closures.
fn scan_block_contexts(source: &str) -> Option<Vec<&'static str>> {
    if source.len() > MAX_CONTEXT_SCAN_BYTES {
        return None;
    }
    let bytes = source.as_bytes();
    let mut stack: Vec<Option<&'static str>> = Vec::new();
    let mut candidate = None;
    let mut parentheses = 0usize;
    let mut brackets = 0usize;
    let mut statement_start = true;
    let mut last_significant = None;
    let mut index = 0usize;

    while index < bytes.len() {
        match bytes[index] {
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index + 1 < bytes.len()
                    && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
                {
                    index += 1;
                }
                index = (index + 2).min(bytes.len());
            }
            b'$' if bytes.get(index + 1) == Some(&b'/') => {
                index = skip_dollar_slashy_string(bytes, index);
                statement_start = false;
                last_significant = Some(b'$');
            }
            b'\'' | b'"' => {
                index = skip_quoted_string(bytes, index);
                statement_start = false;
                last_significant = Some(b'"');
            }
            b'/' if slashy_string_can_start(bytes, index) => {
                index = skip_slashy_string(bytes, index);
                statement_start = false;
                last_significant = Some(b'\"');
            }
            b'\n' => {
                if parentheses == 0 && brackets == 0
                    && !matches!(last_significant, Some(b'.' | b'=' | b',' | b':' | b'(' | b'['
                        | b'+' | b'-' | b'*' | b'/' | b'&' | b'|' | b'?' | b'!')) {
                    statement_start = true;
                    if last_significant == Some(b')') {
                        candidate = None;
                    }
                }
                index += 1;
            }
            byte if byte.is_ascii_whitespace() => index += 1,
            b'{' => {
                let declarative_parent = stack.iter().all(|block| {
                    block.is_some_and(|name| name != "script")
                });
                let opened = if declarative_parent && parentheses == 0 && brackets == 0 {
                    candidate.take()
                } else {
                    None
                };
                stack.push(opened);
                statement_start = true;
                last_significant = Some(b'{');
                index += 1;
            }
            b'}' => {
                stack.pop();
                candidate = None;
                statement_start = true;
                last_significant = Some(b'}');
                index += 1;
            }
            b'(' => {
                parentheses = parentheses.saturating_add(1);
                statement_start = false;
                last_significant = Some(b'(');
                index += 1;
            }
            b')' => {
                parentheses = parentheses.saturating_sub(1);
                statement_start = false;
                last_significant = Some(b')');
                index += 1;
            }
            b'[' => {
                brackets = brackets.saturating_add(1);
                candidate = None;
                statement_start = false;
                last_significant = Some(b'[');
                index += 1;
            }
            b']' => {
                brackets = brackets.saturating_sub(1);
                candidate = None;
                statement_start = false;
                last_significant = Some(b']');
                index += 1;
            }
            b';' => {
                candidate = None;
                statement_start = true;
                last_significant = Some(b';');
                index += 1;
            }
            b':' => {
                candidate = None;
                statement_start = false;
                last_significant = Some(b':');
                index += 1;
            }
            b'.' | b'=' | b'+' | b'-' | b'*' | b'%' | b'!' | b'?' | b'&' | b'|' | b'~' => {
                candidate = None;
                statement_start = false;
                last_significant = Some(bytes[index]);
                index += 1;
            }
            byte if byte.is_ascii_alphabetic() || matches!(byte, b'_' | b'$') => {
                let start = index;
                index += 1;
                while index < bytes.len()
                    && (bytes[index].is_ascii_alphanumeric()
                        || matches!(bytes[index], b'_' | b'$'))
                {
                    index += 1;
                }
                let in_groovy_expression = parentheses > 0 || brackets > 0
                    || stack.iter().any(|block| block.is_none() || *block == Some("script"));
                if statement_start && !in_groovy_expression {
                    candidate = relevant_block(&source[start..index]);
                }
                statement_start = false;
                last_significant = Some(bytes[index - 1]);
            }
            _ => {
                candidate = None;
                statement_start = false;
                last_significant = Some(bytes[index]);
                index += 1;
            }
        }
    }

    Some(stack.into_iter().flatten().collect())
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

    let blocks = scan_block_contexts(&before).unwrap_or_default();
    let in_script = blocks.contains(&"script");
    let in_steps = blocks.contains(&"steps");
    let declarative = blocks.contains(&"pipeline") && !in_script;
    let mut contexts = vec!["pipeline".to_owned()];
    for block in blocks {
        if block != "pipeline" && !contexts.iter().any(|context| context == block) {
            contexts.push(block.to_owned());
        }
    }
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

    static NEXT_PACKAGE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn fixture_runtime(profile: &str) -> Runtime {
        fixture_runtime_with_symbols(profile, Vec::new())
    }

    fn fixture_runtime_with_symbols(profile: &str, extra_symbols: Vec<Value>) -> Runtime {
        let package = std::env::temp_dir().join(format!(
            "jenkins-lsp-test-{}-{}",
            process::id(),
            NEXT_PACKAGE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&package);
        fs::create_dir_all(&package).unwrap();
        let mut catalog: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/catalog-valid.json"
        )).unwrap();
        catalog["symbols"].as_array_mut().unwrap().extend(extra_symbols);
        let catalog_bytes = serde_json::to_vec(&catalog).unwrap();
        fs::write(package.join("catalog.json"), &catalog_bytes).unwrap();
        let checksum = sha2::Sha256::digest(&catalog_bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let mut source_revisions = serde_json::Map::new();
        let mut provenance = serde_json::Map::new();
        for source in catalog["sources"].as_array().into_iter().flatten() {
            let Some(id) = source["id"].as_str() else { continue; };
            source_revisions.insert(id.into(), source["revision"].clone());
            provenance.insert(id.into(), json!({
                "name": source["name"],
                "kind": source["kind"],
                "revision": source["revision"],
                "url": source.get("url").cloned().unwrap_or(Value::Null)
            }));
        }
        fs::write(package.join("manifest.json"), json!({
            "packageVersion": "1.0.0",
            "catalogSchemaVersion": "1.0.0",
            "profile": profile,
            "createdAt": "2026-01-01T00:00:00Z",
            "catalog": "catalog.json",
            "sourceRevisions": source_revisions,
            "provenance": provenance,
            "checksums": {"catalog.json": checksum},
            "toolVersions": {}
        }).to_string()).unwrap();
        let runtime = Runtime::open(&package).unwrap();
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

    fn context_at_end(text: &str) -> (String, Vec<String>, Vec<String>) {
        let line = text.lines().count().saturating_sub(1);
        let character = text.lines().last().unwrap_or_default().encode_utf16().count();
        completion_context(text, line, character)
    }

    #[test]
    fn nested_closures_do_not_close_their_declarative_parent_context() {
        let source = "pipeline {\n stages {\n stage('build') {\n steps {\n script {\n sh 'inside'\n }\n sh";
        let (_, contexts, kinds) = context_at_end(source);
        assert!(contexts.iter().any(|context| context == "pipeline"));
        assert!(contexts.iter().any(|context| context == "stages"));
        assert!(contexts.iter().any(|context| context == "stage"));
        assert!(contexts.iter().any(|context| context == "steps"));
        assert!(!contexts.iter().any(|context| context == "script"));
        assert_eq!(kinds, vec!["step"]);
    }

    #[test]
    fn ignores_block_like_text_in_comments_and_quoted_strings() {
        let source = r#"pipeline {
    // script { steps {
    /* post { stage('fake') { } } */
    stages {
        stage('real') {
            steps {
                def single = 'script { }'
                def double = "post { }"
                def tripleSingle = '''agent { }'''
                def tripleDouble = """environment { }"""
                sh
"#;
        let (_, contexts, _) = context_at_end(source);
        for expected in ["pipeline", "stages", "stage", "steps"] {
            assert!(contexts.iter().any(|context| context == expected), "{contexts:?}");
        }
        for ignored in ["script", "post", "agent", "environment"] {
            assert!(!contexts.iter().any(|context| context == ignored), "{contexts:?}");
        }
    }

    #[test]
    fn escaped_quotes_do_not_end_strings_early() {
        let source = r#"pipeline {
    def double = "steps { \" stage { }"
    def single = 'script { \' post { }'
    steps {
        sh
"#;
        let (_, contexts, _) = context_at_end(source);
        assert!(contexts.iter().any(|context| context == "steps"));
        assert!(!contexts.iter().any(|context| context == "stage"));
        assert!(!contexts.iter().any(|context| context == "script"));
        assert!(!contexts.iter().any(|context| context == "post"));
    }

    #[test]
    fn incomplete_blocks_and_unbalanced_closers_are_handled_safely() {
        let source = "pipeline {\n stages {\n stage('unfinished') {\n steps {\n sh";
        let (_, contexts, kinds) = context_at_end(source);
        assert!(contexts.iter().any(|context| context == "steps"));
        assert_eq!(kinds, vec!["step"]);

        let (_, contexts, _) = context_at_end("}}}} pipeline {");
        assert!(contexts.iter().any(|context| context == "pipeline"));
    }

    #[test]
    fn nested_declarative_contexts_unwind_one_block_at_a_time() {
        let pipeline = "pipeline {\n stages {\n stage('build') {\n steps {\n sh";
        assert_eq!(
            scan_block_contexts(pipeline).unwrap(),
            vec!["pipeline", "stages", "stage", "steps"]
        );
        assert_eq!(
            scan_block_contexts(&format!("{pipeline}\n }}")).unwrap(),
            vec!["pipeline", "stages", "stage"]
        );
        assert_eq!(
            scan_block_contexts(&format!("{pipeline}\n }}\n }}")).unwrap(),
            vec!["pipeline", "stages"]
        );
        assert_eq!(
            scan_block_contexts(&format!("{pipeline}\n }}\n }}\n }}")).unwrap(),
            vec!["pipeline"]
        );
        assert!(scan_block_contexts(&format!("{pipeline}\n }}\n }}\n }}\n }}"))
            .unwrap().is_empty());
    }

    #[test]
    fn context_scan_is_bounded_for_very_large_documents() {
        let source = format!("{}pipeline {{", " ".repeat(MAX_CONTEXT_SCAN_BYTES));
        assert!(scan_block_contexts(&source).is_none());
    }

    #[test]
    fn ignores_directive_text_and_braces_in_slashy_and_dollar_slashy_strings() {
        let source = r#"pipeline {
    stages {
        stage('real') {
            steps {
                def slashy = /script { } stage('fake') { } \/ and \\/\\/ text/
                def dollar = $/post { } pipeline { } escaped $/ slash and $$ sign /$
                def interpolated = "value @@INTERP@@ [script: { steps { } }] }"
                sh
"#.replace("@@INTERP@@", "$" + "{");
        let (_, contexts, _) = context_at_end(source);
        for expected in ["pipeline", "stages", "stage", "steps"] {
            assert!(contexts.iter().any(|context| context == expected), "{contexts:?}");
        }
        for ignored in ["script", "post"] {
            assert!(!contexts.iter().any(|context| context == ignored), "{contexts:?}");
        }
    }

    #[test]
    fn map_list_literals_and_method_calls_do_not_open_declarative_contexts() {
        let source = r#"pipeline {
    stages {
        stage('real') {
            steps {
                def map = [stage: { script { } }, nested: [post: { }]]
                def list = [{ agent { } }, [steps: { }]]
                def value = helper(stage: [script: { }])
                helper.stage()
                def result = stage('not a block')
                sh
"#;
        let (_, contexts, _) = context_at_end(source);
        for expected in ["pipeline", "stages", "stage", "steps"] {
            assert!(contexts.iter().any(|context| context == expected), "{contexts:?}");
        }
        for ignored in ["script", "post", "agent"] {
            assert!(!contexts.iter().any(|context| context == ignored), "{contexts:?}");
        }
    }

    #[test]
    fn ordinary_closures_and_script_bodies_do_not_create_declarative_contexts() {
        let source = r#"pipeline {
    stages {
        stage('real') {
            steps {
                def closure = {
                    stage { script { } }
                }
                script {
                    stage { steps { } }
                }
                sh
"#;
        let (_, contexts, _) = context_at_end(source);
        for expected in ["pipeline", "stages", "stage", "steps"] {
            assert!(contexts.iter().any(|context| context == expected), "{contexts:?}");
        }
        assert!(!contexts.iter().any(|context| context == "script"), "{contexts:?}");
    }

    #[test]
    fn nested_completion_keeps_step_symbols_and_excludes_declarative_directives() {
        let runtime = fixture_runtime("fixture");
        let source = "pipeline {\n stages {\n stage('build') {\n steps {\n script {\n sh 'inside'\n }\n sh";
        let result = completion(Some(&runtime), source, 7, 2);
        let labels: Vec<_> = result["items"].as_array().unwrap().iter()
            .filter_map(|item| item["label"].as_str()).collect();
        assert!(labels.contains(&"sh"));
        assert!(!labels.contains(&"agent"));
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
    #[test]
    fn completion_uses_prefix_and_declarative_or_scripted_context() {
        let runtime = fixture_runtime("fixture");
        let declarative = completion(Some(&runtime), "pipeline {\n  agent ", 1, 8);
        let declarative_labels: Vec<_> = declarative["items"].as_array().unwrap().iter()
            .filter_map(|item| item["label"].as_str()).collect();
        assert!(declarative_labels.contains(&"agent"));

        let scripted = completion(Some(&runtime), "node {\n  sh", 1, 4);
        let scripted_labels: Vec<_> = scripted["items"].as_array().unwrap().iter()
            .filter_map(|item| item["label"].as_str()).collect();
        assert!(scripted_labels.contains(&"sh"));
        assert!(!scripted_labels.contains(&"agent"));
        assert!(scripted_labels.iter().all(|label| label.starts_with("sh")));
    }

    #[test]
    fn completion_finds_prefix_matches_beyond_the_old_global_first_hundred() {
        let mut extra = Vec::new();
        for index in 0..120 {
            extra.push(json!({
                "id": format!("library:aaa-{index:03}"),
                "name": format!("aaa-{index:03}"),
                "kind": "global-variable",
                "sourceId": "platform-lib",
                "contexts": ["pipeline"],
                "confidence": "verified",
                "provenance": ["test"]
            }));
        }
        extra.push(json!({
            "id": "library:zzTarget",
            "name": "zzTarget",
            "kind": "global-variable",
            "sourceId": "platform-lib",
            "contexts": ["pipeline"],
            "confidence": "verified",
            "provenance": ["test"]
        }));
        let runtime = fixture_runtime_with_symbols("fixture", extra);
        let result = completion(Some(&runtime), "node {\n  zz", 1, 4);
        let labels: Vec<_> = result["items"].as_array().unwrap().iter()
            .filter_map(|item| item["label"].as_str()).collect();
        assert!(labels.contains(&"zzTarget"));
        assert_eq!(labels.len(), 1);
        assert_eq!(result["isIncomplete"], false);
    }


}
) || bytes.get(index + 1) == Some(&b'/')) {
            index += 2;
        } else if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'$') {
            return index + 2;
        } else {
            index += 1;
        }
    }
    bytes.len()
}

fn slashy_string_can_start(bytes: &[u8], index: usize) -> bool {
    let mut previous = index;
    while previous > 0 && bytes[previous - 1].is_ascii_whitespace() {
        previous -= 1;
    }
    if previous == 0 {
        return true;
    }
    if matches!(bytes[previous - 1], b'=' | b'(' | b'[' | b'{' | b',' | b':' | b';'
        | b'!' | b'?' | b'&' | b'|' | b'~' | b'>') {
        return true;
    }
    let end = previous;
    while previous > 0 && (bytes[previous - 1].is_ascii_alphanumeric()
        || matches!(bytes[previous - 1], b'_' | b'$')) {
        previous -= 1;
    }
    matches!(&bytes[previous..end], b"return" | b"throw" | b"case" | b"in" | b"assert")
}

/// Return the relevant Declarative block stack before the cursor. This is a
/// bounded lexical recognizer, not a Groovy parser: it skips comments and all
/// Groovy string literal forms and only accepts block names at statement starts
/// outside map/list literals and ordinary Groovy closures.
fn scan_block_contexts(source: &str) -> Option<Vec<&'static str>> {
    if source.len() > MAX_CONTEXT_SCAN_BYTES {
        return None;
    }
    let bytes = source.as_bytes();
    let mut stack: Vec<Option<&'static str>> = Vec::new();
    let mut candidate = None;
    let mut parentheses = 0usize;
    let mut brackets = 0usize;
    let mut statement_start = true;
    let mut last_significant = None;
    let mut index = 0usize;

    while index < bytes.len() {
        match bytes[index] {
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index + 1 < bytes.len()
                    && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
                {
                    index += 1;
                }
                index = (index + 2).min(bytes.len());
            }
            b'$' if bytes.get(index + 1) == Some(&b'/') => {
                index = skip_dollar_slashy_string(bytes, index);
                statement_start = false;
                last_significant = Some(b'$');
            }
            b'\'' | b'"' => {
                index = skip_quoted_string(bytes, index);
                statement_start = false;
                last_significant = Some(b'"');
            }
            b'/' if slashy_string_can_start(bytes, index) => {
                index = skip_slashy_string(bytes, index);
                statement_start = false;
                last_significant = Some(b'/');
            }
            b'\n' => {
                if parentheses == 0 && brackets == 0
                    && !matches!(last_significant, Some(b'.' | b'=' | b',' | b':' | b'(' | b'['
                        | b'+' | b'-' | b'*' | b'/' | b'&' | b'|' | b'?' | b'!')) {
                    statement_start = true;
                    if last_significant == Some(b')') {
                        candidate = None;
                    }
                }
                index += 1;
            }
            byte if byte.is_ascii_whitespace() => index += 1,
            b'{' => {
                let declarative_parent = stack.iter().all(|block| {
                    block.is_some_and(|name| name != "script")
                });
                let opened = if declarative_parent && parentheses == 0 && brackets == 0 {
                    candidate.take()
                } else {
                    None
                };
                stack.push(opened);
                statement_start = true;
                last_significant = Some(b'{');
                index += 1;
            }
            b'}' => {
                stack.pop();
                candidate = None;
                statement_start = true;
                last_significant = Some(b'}');
                index += 1;
            }
            b'(' => {
                parentheses = parentheses.saturating_add(1);
                statement_start = false;
                last_significant = Some(b'(');
                index += 1;
            }
            b')' => {
                parentheses = parentheses.saturating_sub(1);
                statement_start = false;
                last_significant = Some(b')');
                index += 1;
            }
            b'[' => {
                brackets = brackets.saturating_add(1);
                candidate = None;
                statement_start = false;
                last_significant = Some(b'[');
                index += 1;
            }
            b']' => {
                brackets = brackets.saturating_sub(1);
                candidate = None;
                statement_start = false;
                last_significant = Some(b']');
                index += 1;
            }
            b';' => {
                candidate = None;
                statement_start = true;
                last_significant = Some(b';');
                index += 1;
            }
            b':' => {
                candidate = None;
                statement_start = false;
                last_significant = Some(b':');
                index += 1;
            }
            b'.' | b'=' | b'+' | b'-' | b'*' | b'%' | b'!' | b'?' | b'&' | b'|' | b'~' => {
                candidate = None;
                statement_start = false;
                last_significant = Some(bytes[index]);
                index += 1;
            }
            byte if byte.is_ascii_alphabetic() || matches!(byte, b'_' | b'$') => {
                let start = index;
                index += 1;
                while index < bytes.len()
                    && (bytes[index].is_ascii_alphanumeric()
                        || matches!(bytes[index], b'_' | b'$'))
                {
                    index += 1;
                }
                let in_groovy_expression = parentheses > 0 || brackets > 0
                    || stack.iter().any(|block| block.is_none() || *block == Some("script"));
                if statement_start && !in_groovy_expression {
                    candidate = relevant_block(&source[start..index]);
                }
                statement_start = false;
                last_significant = Some(bytes[index - 1]);
            }
            _ => {
                candidate = None;
                statement_start = false;
                last_significant = Some(bytes[index]);
                index += 1;
            }
        }
    }

    Some(stack.into_iter().flatten().collect())
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

    let blocks = scan_block_contexts(&before).unwrap_or_default();
    let in_script = blocks.contains(&"script");
    let in_steps = blocks.contains(&"steps");
    let declarative = blocks.contains(&"pipeline") && !in_script;
    let mut contexts = vec!["pipeline".to_owned()];
    for block in blocks {
        if block != "pipeline" && !contexts.iter().any(|context| context == block) {
            contexts.push(block.to_owned());
        }
    }
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

    static NEXT_PACKAGE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn fixture_runtime(profile: &str) -> Runtime {
        fixture_runtime_with_symbols(profile, Vec::new())
    }

    fn fixture_runtime_with_symbols(profile: &str, extra_symbols: Vec<Value>) -> Runtime {
        let package = std::env::temp_dir().join(format!(
            "jenkins-lsp-test-{}-{}",
            process::id(),
            NEXT_PACKAGE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&package);
        fs::create_dir_all(&package).unwrap();
        let mut catalog: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/catalog-valid.json"
        )).unwrap();
        catalog["symbols"].as_array_mut().unwrap().extend(extra_symbols);
        let catalog_bytes = serde_json::to_vec(&catalog).unwrap();
        fs::write(package.join("catalog.json"), &catalog_bytes).unwrap();
        let checksum = sha2::Sha256::digest(&catalog_bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let mut source_revisions = serde_json::Map::new();
        let mut provenance = serde_json::Map::new();
        for source in catalog["sources"].as_array().into_iter().flatten() {
            let Some(id) = source["id"].as_str() else { continue; };
            source_revisions.insert(id.into(), source["revision"].clone());
            provenance.insert(id.into(), json!({
                "name": source["name"],
                "kind": source["kind"],
                "revision": source["revision"],
                "url": source.get("url").cloned().unwrap_or(Value::Null)
            }));
        }
        fs::write(package.join("manifest.json"), json!({
            "packageVersion": "1.0.0",
            "catalogSchemaVersion": "1.0.0",
            "profile": profile,
            "createdAt": "2026-01-01T00:00:00Z",
            "catalog": "catalog.json",
            "sourceRevisions": source_revisions,
            "provenance": provenance,
            "checksums": {"catalog.json": checksum},
            "toolVersions": {}
        }).to_string()).unwrap();
        let runtime = Runtime::open(&package).unwrap();
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

    fn context_at_end(text: &str) -> (String, Vec<String>, Vec<String>) {
        let line = text.lines().count().saturating_sub(1);
        let character = text.lines().last().unwrap_or_default().encode_utf16().count();
        completion_context(text, line, character)
    }

    #[test]
    fn nested_closures_do_not_close_their_declarative_parent_context() {
        let source = "pipeline {\n stages {\n stage('build') {\n steps {\n script {\n sh 'inside'\n }\n sh";
        let (_, contexts, kinds) = context_at_end(source);
        assert!(contexts.iter().any(|context| context == "pipeline"));
        assert!(contexts.iter().any(|context| context == "stages"));
        assert!(contexts.iter().any(|context| context == "stage"));
        assert!(contexts.iter().any(|context| context == "steps"));
        assert!(!contexts.iter().any(|context| context == "script"));
        assert_eq!(kinds, vec!["step"]);
    }

    #[test]
    fn ignores_block_like_text_in_comments_and_quoted_strings() {
        let source = r#"pipeline {
    // script { steps {
    /* post { stage('fake') { } } */
    stages {
        stage('real') {
            steps {
                def single = 'script { }'
                def double = "post { }"
                def tripleSingle = '''agent { }'''
                def tripleDouble = """environment { }"""
                sh
"#;
        let (_, contexts, _) = context_at_end(source);
        for expected in ["pipeline", "stages", "stage", "steps"] {
            assert!(contexts.iter().any(|context| context == expected), "{contexts:?}");
        }
        for ignored in ["script", "post", "agent", "environment"] {
            assert!(!contexts.iter().any(|context| context == ignored), "{contexts:?}");
        }
    }

    #[test]
    fn escaped_quotes_do_not_end_strings_early() {
        let source = r#"pipeline {
    def double = "steps { \" stage { }"
    def single = 'script { \' post { }'
    steps {
        sh
"#;
        let (_, contexts, _) = context_at_end(source);
        assert!(contexts.iter().any(|context| context == "steps"));
        assert!(!contexts.iter().any(|context| context == "stage"));
        assert!(!contexts.iter().any(|context| context == "script"));
        assert!(!contexts.iter().any(|context| context == "post"));
    }

    #[test]
    fn incomplete_blocks_and_unbalanced_closers_are_handled_safely() {
        let source = "pipeline {\n stages {\n stage('unfinished') {\n steps {\n sh";
        let (_, contexts, kinds) = context_at_end(source);
        assert!(contexts.iter().any(|context| context == "steps"));
        assert_eq!(kinds, vec!["step"]);

        let (_, contexts, _) = context_at_end("}}}} pipeline {");
        assert!(contexts.iter().any(|context| context == "pipeline"));
    }

    #[test]
    fn context_scan_is_bounded_for_very_large_documents() {
        let source = format!("{}pipeline {{", " ".repeat(MAX_CONTEXT_SCAN_BYTES));
        assert!(scan_block_contexts(&source).is_none());
    }

    #[test]
    fn ignores_directive_text_and_braces_in_slashy_and_dollar_slashy_strings() {
        let source = r#"pipeline {
    stages {
        stage('real') {
            steps {
                def slashy = /script { } stage('fake') { } \\/ and // text/
                def dollar = $/post { } pipeline { } escaped $/ slash and $$ sign /$
                def interpolated = "value @@INTERP@@ [script: { steps { } }] }"
                sh
"#.replace("@@INTERP@@", "$" + "{");
        let (_, contexts, _) = context_at_end(source);
        for expected in ["pipeline", "stages", "stage", "steps"] {
            assert!(contexts.iter().any(|context| context == expected), "{contexts:?}");
        }
        for ignored in ["script", "post"] {
            assert!(!contexts.iter().any(|context| context == ignored), "{contexts:?}");
        }
    }

    #[test]
    fn map_list_literals_and_method_calls_do_not_open_declarative_contexts() {
        let source = r#"pipeline {
    stages {
        stage('real') {
            steps {
                def map = [stage: { script { } }, nested: [post: { }]]
                def list = [{ agent { } }, [steps: { }]]
                def value = helper(stage: [script: { }])
                helper.stage()
                sh
"#;
        let (_, contexts, _) = context_at_end(source);
        for expected in ["pipeline", "stages", "stage", "steps"] {
            assert!(contexts.iter().any(|context| context == expected), "{contexts:?}");
        }
        for ignored in ["script", "post", "agent"] {
            assert!(!contexts.iter().any(|context| context == ignored), "{contexts:?}");
        }
    }

    #[test]
    fn ordinary_closures_and_script_bodies_do_not_create_declarative_contexts() {
        let source = r#"pipeline {
    stages {
        stage('real') {
            steps {
                def closure = {
                    stage { script { } }
                }
                script {
                    stage { steps { } }
                }
                sh
"#;
        let (_, contexts, _) = context_at_end(source);
        for expected in ["pipeline", "stages", "stage", "steps"] {
            assert!(contexts.iter().any(|context| context == expected), "{contexts:?}");
        }
        assert!(!contexts.iter().any(|context| context == "script"), "{contexts:?}");
    }

    #[test]
    fn nested_completion_keeps_step_symbols_and_excludes_declarative_directives() {
        let runtime = fixture_runtime("fixture");
        let source = "pipeline {\n stages {\n stage('build') {\n steps {\n script {\n sh 'inside'\n }\n sh";
        let result = completion(Some(&runtime), source, 7, 2);
        let labels: Vec<_> = result["items"].as_array().unwrap().iter()
            .filter_map(|item| item["label"].as_str()).collect();
        assert!(labels.contains(&"sh"));
        assert!(!labels.contains(&"agent"));
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
    #[test]
    fn completion_uses_prefix_and_declarative_or_scripted_context() {
        let runtime = fixture_runtime("fixture");
        let declarative = completion(Some(&runtime), "pipeline {\n  agent ", 1, 8);
        let declarative_labels: Vec<_> = declarative["items"].as_array().unwrap().iter()
            .filter_map(|item| item["label"].as_str()).collect();
        assert!(declarative_labels.contains(&"agent"));

        let scripted = completion(Some(&runtime), "node {\n  sh", 1, 4);
        let scripted_labels: Vec<_> = scripted["items"].as_array().unwrap().iter()
            .filter_map(|item| item["label"].as_str()).collect();
        assert!(scripted_labels.contains(&"sh"));
        assert!(!scripted_labels.contains(&"agent"));
        assert!(scripted_labels.iter().all(|label| label.starts_with("sh")));
    }

    #[test]
    fn completion_finds_prefix_matches_beyond_the_old_global_first_hundred() {
        let mut extra = Vec::new();
        for index in 0..120 {
            extra.push(json!({
                "id": format!("library:aaa-{index:03}"),
                "name": format!("aaa-{index:03}"),
                "kind": "global-variable",
                "sourceId": "platform-lib",
                "contexts": ["pipeline"],
                "confidence": "verified",
                "provenance": ["test"]
            }));
        }
        extra.push(json!({
            "id": "library:zzTarget",
            "name": "zzTarget",
            "kind": "global-variable",
            "sourceId": "platform-lib",
            "contexts": ["pipeline"],
            "confidence": "verified",
            "provenance": ["test"]
        }));
        let runtime = fixture_runtime_with_symbols("fixture", extra);
        let result = completion(Some(&runtime), "node {\n  zz", 1, 4);
        let labels: Vec<_> = result["items"].as_array().unwrap().iter()
            .filter_map(|item| item["label"].as_str()).collect();
        assert!(labels.contains(&"zzTarget"));
        assert_eq!(labels.len(), 1);
        assert_eq!(result["isIncomplete"], false);
    }


}
