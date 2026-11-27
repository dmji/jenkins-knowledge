use jenkins_runtime::{Runtime, SearchOptions};
use knowledge_builder::build_package;
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{ChildStdin, ChildStdout, Command, Stdio},
};

fn fixture_root(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn copy_fixture(name: &str, destination: &Path, probe_url: Option<&str>) {
    fs::create_dir_all(destination).unwrap();
    for file in ["profile.yml", "catalog.json", "manifest.json"] {
        let source = fixture_root(name).join(file);
        if !source.exists() {
            continue;
        }
        let mut content = fs::read_to_string(source).unwrap();
        if file == "catalog.json" {
            if let Some(url) = probe_url {
                content = content.replace("http://127.0.0.1:9/", url);
            }
        }
        fs::write(destination.join(file), content).unwrap();
    }
}

fn lsp_write(writer: &mut ChildStdin, message: &Value) {
    let body = serde_json::to_vec(message).unwrap();
    write!(writer, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
    writer.write_all(&body).unwrap();
    writer.flush().unwrap();
}

fn lsp_read(reader: &mut BufReader<ChildStdout>) -> Value {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        assert_ne!(
            reader.read_line(&mut line).unwrap(),
            0,
            "LSP closed stdout unexpectedly"
        );
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = Some(value.trim().parse::<usize>().unwrap());
            }
        }
    }
    let mut body = vec![0; content_length.expect("LSP response has Content-Length")];
    reader.read_exact(&mut body).unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn lsp_response(reader: &mut BufReader<ChildStdout>, id: u64) -> Value {
    loop {
        let message = lsp_read(reader);
        if message.get("id") == Some(&json!(id)) {
            return message;
        }
    }
}

fn lsp_response_with_notifications(
    reader: &mut BufReader<ChildStdout>,
    id: u64,
) -> (Value, Vec<Value>) {
    let mut notifications = Vec::new();
    loop {
        let message = lsp_read(reader);
        if message.get("id") == Some(&json!(id)) {
            return (message, notifications);
        }
        if message.get("method").and_then(Value::as_str) == Some("textDocument/publishDiagnostics")
        {
            notifications.push(message);
        }
    }
}

fn mcp_request(
    writer: &mut ChildStdin,
    reader: &mut BufReader<ChildStdout>,
    id: u64,
    method: &str,
    params: Value,
) -> Value {
    writeln!(
        writer,
        "{}",
        json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
    )
    .unwrap();
    writer.flush().unwrap();
    let mut line = String::new();
    assert_ne!(
        reader.read_line(&mut line).unwrap(),
        0,
        "MCP closed stdout unexpectedly"
    );
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["id"], id);
    response
}

fn run_lsp(primary: &Path, alternate: &Path, expected: &Value) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_contract_lsp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut writer = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());

    lsp_write(
        &mut writer,
        &json!({
            "jsonrpc":"2.0",
            "id":1,
            "method":"initialize",
            "params":{"initializationOptions":{
                "packagePath":primary.to_string_lossy(),
                "profile":"contract-primary"
            }}
        }),
    );
    let initialized = lsp_response(&mut reader, 1);
    assert!(initialized["result"]["capabilities"]["hoverProvider"] == true);
    lsp_write(
        &mut writer,
        &json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
    );

    let uri = "file:///workspace/Jenkinsfile";
    lsp_write(
        &mut writer,
        &json!({
            "jsonrpc":"2.0",
            "method":"textDocument/didOpen",
            "params":{"textDocument":{
                "uri":uri,
                "languageId":"groovy",
                "version":1,
                "text":"node {\n  sh 'echo ok'\n  unavailableStep()\n  deployApp()\n}\n"
            }}
        }),
    );

    lsp_write(
        &mut writer,
        &json!({
            "jsonrpc":"2.0",
            "id":2,
            "method":"textDocument/completion",
            "params":{"textDocument":{"uri":uri},"position":{"line":1,"character":4}}
        }),
    );
    let (completion, notifications) = lsp_response_with_notifications(&mut reader, 2);
    let diagnostics = notifications
        .iter()
        .filter(|message| message["method"] == "textDocument/publishDiagnostics")
        .flat_map(|message| {
            message["params"]["diagnostics"]
                .as_array()
                .into_iter()
                .flatten()
        });
    assert!(diagnostics
        .into_iter()
        .any(|diagnostic| diagnostic["code"] == "jenkins.plugin.unavailable"));
    let labels: Vec<_> = completion["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["label"].as_str())
        .collect();
    assert!(labels.contains(&"sh"));

    let ordinary_uri = "file:///workspace/Ordinary.groovy";
    lsp_write(
        &mut writer,
        &json!({
            "jsonrpc":"2.0",
            "method":"textDocument/didOpen",
            "params":{"textDocument":{
                "uri":ordinary_uri,
                "languageId":"groovy",
                "version":1,
                "text":"node {\\n  deployApp()\\n}\\n"
            }}
        }),
    );
    lsp_write(
        &mut writer,
        &json!({
            "jsonrpc":"2.0",
            "id":12,
            "method":"textDocument/completion",
            "params":{"textDocument":{"uri":ordinary_uri},"position":{"line":1,"character":8}}
        }),
    );
    assert!(lsp_response(&mut reader, 12)["result"]["items"].as_array().unwrap().is_empty());

    lsp_write(
        &mut writer,
        &json!({
            "jsonrpc":"2.0",
            "id":3,
            "method":"textDocument/hover",
            "params":{"textDocument":{"uri":uri},"position":{"line":3,"character":6}}
        }),
    );
    let hover = lsp_response(&mut reader, 3);
    let hover_text = hover["result"]["contents"]["value"].as_str().unwrap();
    assert!(hover_text.contains("deployApp"));
    assert!(hover_text.contains("verified"));
    assert!(hover_text.contains("Fixture Shared Library"));

    lsp_write(
        &mut writer,
        &json!({
            "jsonrpc":"2.0",
            "id":10,
            "method":"textDocument/definition",
            "params":{"textDocument":{"uri":uri},"position":{"line":3,"character":6}}
        }),
    );
    let definition = lsp_response(&mut reader, 10);
    assert!(
        definition["result"].is_null(),
        "non-GitHub fixture URLs must not become navigation targets"
    );

    lsp_write(
        &mut writer,
        &json!({
            "jsonrpc":"2.0",
            "id":11,
            "method":"textDocument/definition",
            "params":{"textDocument":{"uri":uri},"position":{"line":2,"character":8}}
        }),
    );
    let missing_definition = lsp_response(&mut reader, 11);
    assert!(missing_definition["result"].is_null());

    lsp_write(
        &mut writer,
        &json!({
            "jsonrpc":"2.0",
            "id":4,
            "method":"jenkinsKnowledge/search",
            "params":{"query":"deployApp","limit":10}
        }),
    );
    let search = lsp_response(&mut reader, 4);
    let result = &search["result"]["results"][0];
    assert_eq!(result["id"], expected["id"]);
    assert_eq!(result["sourceId"], expected["sourceId"]);
    assert_eq!(result["confidence"], expected["confidence"]);
    assert_eq!(result["source"]["revision"], expected["source"]["revision"]);

    lsp_write(
        &mut writer,
        &json!({"jsonrpc":"2.0","id":5,"method":"jenkinsKnowledge/collection","params":{}}),
    );
    let collection = lsp_response(&mut reader, 5);
    assert_eq!(collection["result"]["status"], "partial");
    assert!(collection["result"]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .any(|source| source["status"] == "succeeded-with-truncation"));

    lsp_write(
        &mut writer,
        &json!({"jsonrpc":"2.0","id":9,"method":"jenkinsKnowledge/controller","params":{}}),
    );
    let controller = lsp_response(&mut reader, 9);
    assert_eq!(
        controller["result"]["controller"]["collection"]["javaVersionStatus"],
        "available"
    );
    assert_eq!(
        controller["result"]["controller"]["collection"]["javaVendorStatus"],
        "available"
    );

    lsp_write(
        &mut writer,
        &json!({
            "jsonrpc":"2.0",
            "method":"workspace/didChangeConfiguration",
            "params":{"settings":{"jenkinsKnowledge":{
                "packagePath":alternate.to_string_lossy(),
                "profile":"contract-alternate"
            }}}
        }),
    );
    lsp_write(
        &mut writer,
        &json!({
            "jsonrpc":"2.0",
            "id":6,
            "method":"jenkinsKnowledge/search",
            "params":{"query":"deployApp","limit":10}
        }),
    );
    let after_switch = lsp_response(&mut reader, 6);
    assert!(after_switch["result"]["results"]
        .as_array()
        .unwrap()
        .is_empty());

    lsp_write(
        &mut writer,
        &json!({
            "jsonrpc":"2.0",
            "id":7,
            "method":"jenkinsKnowledge/search",
            "params":{"query":"alternateOnly","limit":10}
        }),
    );
    let alternate_result = lsp_response(&mut reader, 7);
    assert_eq!(alternate_result["result"]["profile"], "contract-alternate");
    assert_eq!(
        alternate_result["result"]["results"][0]["name"],
        "alternateOnly"
    );

    lsp_write(
        &mut writer,
        &json!({"jsonrpc":"2.0","id":8,"method":"shutdown","params":null}),
    );
    assert!(lsp_response(&mut reader, 8)["result"].is_null());
    lsp_write(
        &mut writer,
        &json!({"jsonrpc":"2.0","method":"exit","params":null}),
    );
    drop(writer);
    assert!(child.wait().unwrap().success());
}

fn run_mcp(package: &Path, expected: &Value) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_contract_mcp"))
        .arg("--package")
        .arg(package)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut writer = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());

    let initialized = mcp_request(&mut writer, &mut reader, 1, "initialize", json!({}));
    assert_eq!(
        initialized["result"]["serverInfo"]["name"],
        "jenkins-knowledge"
    );

    let tools = mcp_request(&mut writer, &mut reader, 2, "tools/list", json!({}));
    let tool_names: Vec<_> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(tool_names.contains(&"search_api"));
    assert!(tool_names.contains(&"get_api_details"));
    assert!(tool_names.contains(&"get_pipeline_step"));
    assert!(tool_names.contains(&"get_controller_inventory"));

    let search = mcp_request(
        &mut writer,
        &mut reader,
        3,
        "tools/call",
        json!({"name":"search_api","arguments":{"query":"deployApp","limit":10}}),
    );
    let result = &search["result"]["structuredContent"]["results"][0];
    assert_eq!(result["id"], expected["id"]);
    assert_eq!(result["sourceId"], expected["sourceId"]);
    assert_eq!(result["confidence"], expected["confidence"]);
    assert_eq!(result["source"]["revision"], expected["source"]["revision"]);

    let details = mcp_request(
        &mut writer,
        &mut reader,
        4,
        "tools/call",
        json!({"name":"get_api_details","arguments":{"name_or_id":"deployApp"}}),
    );
    let details = &details["result"]["structuredContent"];
    assert_eq!(details["source"]["id"], expected["source"]["id"]);
    assert_eq!(
        details["evidence"][0]["revision"],
        expected["evidence"][0]["revision"]
    );
    assert_eq!(details["confidence"], "verified");

    let ambiguous = mcp_request(
        &mut writer,
        &mut reader,
        5,
        "tools/call",
        json!({"name":"get_api_details","arguments":{"name_or_id":"build"}}),
    );
    assert_eq!(ambiguous["result"]["isError"], true);

    let collection = mcp_request(
        &mut writer,
        &mut reader,
        6,
        "resources/read",
        json!({"uri":"jenkins://catalog/collection"}),
    );
    let collection_text = collection["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let collection: Value = serde_json::from_str(collection_text).unwrap();
    assert_eq!(collection["status"], "partial");
    assert!(collection["sources"]
        .as_array()
        .unwrap()
        .iter()
        .any(|source| source["status"] == "permission-denied"));

    let inventory = mcp_request(
        &mut writer,
        &mut reader,
        7,
        "tools/call",
        json!({"name":"get_controller_inventory","arguments":{}}),
    );
    let inventory = &inventory["result"]["structuredContent"];
    assert_eq!(inventory["profile"], "contract-primary");
    assert_eq!(
        inventory["controller"]["identity"]["id"],
        "controller-fixture"
    );
    assert_eq!(inventory["controller"]["coreVersion"], "2.500");
    assert_eq!(
        inventory["controller"]["collection"]["javaVersionStatus"],
        "available"
    );
    assert_eq!(
        inventory["controller"]["collection"]["javaVendorStatus"],
        "available"
    );
    assert_eq!(inventory["plugins"]["status"], "unknown");
    assert_eq!(inventory["plugins"]["items"].as_array().unwrap().len(), 2);
    assert_eq!(inventory["capabilities"]["status"], "known");
    assert_eq!(inventory["collection"]["controller"]["profile"], "deep");
    assert_eq!(inventory["collection"]["package"]["status"], "partial");
    assert_eq!(
        inventory["freshness"]["controllerObservedAt"],
        "2026-01-01T00:00:00Z"
    );
    assert!(inventory["provenance"]["controllerSource"]["manifestProvenance"].is_object());

    let inventory_resource = mcp_request(
        &mut writer,
        &mut reader,
        8,
        "resources/read",
        json!({"uri":"jenkins://catalog/controller-inventory"}),
    );
    let inventory_text = inventory_resource["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let inventory_resource: Value = serde_json::from_str(inventory_text).unwrap();
    assert_eq!(inventory_resource["plugins"]["status"], "unknown");

    let type_query = mcp_request(
        &mut writer,
        &mut reader,
        9,
        "tools/call",
        json!({"name":"search_api","arguments":{
            "query":"getInstance",
            "type":"jenkins.model.Jenkins",
            "limit":10
        }}),
    );
    assert_eq!(
        type_query["result"]["structuredContent"]["results"][0]["name"],
        "getInstance"
    );

    drop(writer);
    assert!(child.wait().unwrap().success());
}

#[test]
fn one_builder_package_keeps_contracts_across_runtime_lsp_and_mcp() {
    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    probe.set_nonblocking(true).unwrap();
    let probe_url = format!("http://{}/", probe.local_addr().unwrap());

    let workspace = tempfile::tempdir().unwrap();
    let primary_fixture = workspace.path().join("primary-fixture");
    let alternate_fixture = workspace.path().join("alternate-fixture");
    copy_fixture("primary", &primary_fixture, Some(&probe_url));
    copy_fixture("alternate", &alternate_fixture, None);

    let primary_package = workspace.path().join("primary-package");
    let alternate_package = workspace.path().join("alternate-package");
    build_package(&primary_fixture.join("profile.yml"), &primary_package).unwrap();
    build_package(&alternate_fixture.join("profile.yml"), &alternate_package).unwrap();

    let runtime = Runtime::open(&primary_package).unwrap();
    assert_eq!(runtime.profile(), "contract-primary");
    assert_eq!(runtime.catalog()["completeness"]["status"], "partial");
    assert_eq!(runtime.collection().unwrap()["status"], "partial");
    let declarative_symbols = runtime.catalog()["symbols"].as_array().unwrap();
    assert!(declarative_symbols.iter().any(|symbol| {
        symbol["extensionCategory"] == "agent"
            && symbol["name"] == "kubernetes"
            && symbol["confidence"] == "verified"
    }));
    assert!(declarative_symbols.iter().any(|symbol| {
        symbol["extensionCategory"] == "parameter"
            && symbol["nameKind"] == "implementation-class"
            && symbol["confidence"] == "unknown"
    }));
    let api_symbols = runtime.catalog()["symbols"].as_array().unwrap();
    assert!(api_symbols
        .iter()
        .any(|symbol| { symbol["kind"] == "class" && symbol["name"] == "hudson.model.Run" }));
    assert!(api_symbols
        .iter()
        .any(|symbol| { symbol["kind"] == "method" && symbol["name"] == "getNumber" }));

    let deploy = runtime.get("deployApp").unwrap();
    assert_eq!(deploy["sourceId"], "shared-library-fixture");
    assert_eq!(deploy["confidence"], "verified");
    assert_eq!(
        deploy["source"]["revision"],
        "0123456789012345678901234567890123456789"
    );
    assert_eq!(
        deploy["evidence"][0]["revision"],
        "0123456789012345678901234567890123456789"
    );

    assert!(runtime
        .get("build")
        .unwrap_err()
        .to_string()
        .contains("ambiguous"));
    assert_eq!(runtime.search("build", Some("step"), None).len(), 2);
    assert_eq!(runtime.get("maybeStep").unwrap()["confidence"], "unknown");
    assert_eq!(
        runtime
            .search("deploy", Some("method"), Some("shared-library-fixture"))
            .len(),
        2
    );

    let type_members = runtime.search_with_options(&SearchOptions {
        query: "getInstance".into(),
        type_name: Some("jenkins.model.Jenkins".into()),
        limit: 10,
        ..SearchOptions::default()
    });
    assert_eq!(type_members.len(), 1);
    assert_eq!(type_members[0]["name"], "getInstance");

    let catalog_before = fs::read(primary_package.join("catalog.json")).unwrap();
    let manifest_before = fs::read(primary_package.join("manifest.json")).unwrap();

    run_lsp(&primary_package, &alternate_package, &deploy);
    run_mcp(&primary_package, &deploy);

    assert_eq!(
        fs::read(primary_package.join("catalog.json")).unwrap(),
        catalog_before
    );
    assert_eq!(
        fs::read(primary_package.join("manifest.json")).unwrap(),
        manifest_before
    );

    match probe.accept() {
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
        Ok((mut stream, _)) => {
            let _ = stream.write_all(b"unexpected consumer network request");
            panic!("Runtime, LSP, or MCP attempted to access the controller URL");
        }
        Err(error) => panic!("could not verify offline consumer behavior: {error}"),
    }
}

#[test]
fn offline_fixture_profiles_build_quick_standard_and_deep_packages() {
    let workspace = tempfile::tempdir().unwrap();
    for profile in ["quick", "standard", "deep"] {
        let fixture = workspace.path().join(format!("{profile}-fixture"));
        let package = workspace.path().join(format!("{profile}-package"));
        copy_fixture("primary", &fixture, None);

        let profile_path = fixture.join("profile.yml");
        let mut profile_text = fs::read_to_string(&profile_path).unwrap();
        profile_text = profile_text.replace(
            "collectionProfile: deep",
            &format!("collectionProfile: {profile}"),
        );
        fs::write(&profile_path, profile_text).unwrap();

        let catalog_path = fixture.join("catalog.json");
        let mut catalog: Value = serde_json::from_slice(&fs::read(&catalog_path).unwrap()).unwrap();
        let manifest_path = fixture.join("manifest.json");
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();

        if profile == "quick" {
            catalog["sources"] = json!(catalog["sources"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|source| source["id"] != "shared-library-fixture")
                .cloned()
                .collect::<Vec<_>>());
            catalog["symbols"] = json!(catalog["symbols"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|symbol| symbol["sourceId"] == "declarative-model-fixture")
                .cloned()
                .collect::<Vec<_>>());
            catalog["coverage"] = json!(catalog["coverage"]
                .as_array()
                .unwrap_or(&Vec::new())
                .iter()
                .filter(|entry| entry["sourceId"] != "shared-library-fixture")
                .cloned()
                .collect::<Vec<_>>());
        } else if profile == "standard" {
            catalog["symbols"] = json!(catalog["symbols"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|symbol| symbol["sourceId"] != "controller-fixture"
                    && symbol["sourceId"] != "jenkins-core-fixture")
                .cloned()
                .collect::<Vec<_>>());
        }

        if profile != "deep" {
            if let Some(controller) = catalog.get_mut("controller").and_then(Value::as_object_mut) {
                controller.remove("runtime");
                controller["collection"]["profile"] = json!(profile);
                controller["collection"]["requestCount"] =
                    json!(if profile == "quick" { 3 } else { 4 });
            }
            for outcome in manifest["collection"]["sources"].as_array_mut().unwrap() {
                let surface = outcome["surface"].as_str().unwrap_or_default();
                let skip = surface == "script-console"
                    || (profile == "quick"
                        && (surface == "pipeline-gdsl" || surface == "library-source"));
                if skip {
                    outcome["status"] = json!("skipped-by-profile");
                    if let Some(fields) = outcome.as_object_mut() {
                        fields.remove("sourceId");
                        fields.remove("revision");
                        fields.remove("errorCode");
                    }
                }
            }
        }
        manifest["collection"]["profile"] = json!(profile);
        fs::write(&catalog_path, serde_json::to_vec_pretty(&catalog).unwrap()).unwrap();
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();

        build_package(&profile_path, &package).unwrap();
        let runtime = Runtime::open(&package).unwrap();
        assert_eq!(runtime.manifest()["collection"]["profile"], profile);
        assert_eq!(runtime.catalog()["completeness"]["status"], "partial");
        let symbols = runtime.symbols();
        match profile {
            "quick" => {
                assert_eq!(symbols.len(), 3);
                assert!(symbols
                    .iter()
                    .all(|symbol| symbol["kind"] == "declarative-directive"));
                assert!(runtime.collection().unwrap()["sources"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|source| source["surface"] == "pipeline-gdsl"
                        && source["status"] == "skipped-by-profile"));
            }
            "standard" => {
                assert_eq!(symbols.len(), 12);
                assert!(symbols.iter().any(|symbol| symbol["kind"] == "step"));
                assert!(symbols
                    .iter()
                    .any(|symbol| symbol["sourceId"] == "shared-library-fixture"));
                assert!(!symbols
                    .iter()
                    .any(|symbol| symbol["sourceId"] == "controller-fixture"));
            }
            "deep" => {
                assert_eq!(symbols.len(), 15);
                assert_eq!(
                    symbols
                        .iter()
                        .filter(|symbol| symbol["confidence"] == "verified")
                        .count(),
                    11
                );
                assert_eq!(
                    symbols
                        .iter()
                        .filter(|symbol| symbol["confidence"] == "inferred")
                        .count(),
                    2
                );
                assert_eq!(
                    symbols
                        .iter()
                        .filter(|symbol| symbol["confidence"] == "unknown")
                        .count(),
                    2
                );
                assert!(symbols
                    .iter()
                    .any(|symbol| symbol["sourceId"] == "controller-fixture"));
                assert!(runtime.catalog()["controller"]["runtime"].is_object());
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn builder_rejects_a_fixture_with_a_dangling_symbol_source() {
    let workspace = tempfile::tempdir().unwrap();
    let fixture = workspace.path().join("fixture");
    copy_fixture("primary", &fixture, None);

    let catalog_path = fixture.join("catalog.json");
    let mut catalog: Value = serde_json::from_slice(&fs::read(&catalog_path).unwrap()).unwrap();
    catalog["symbols"][4]["sourceId"] = json!("missing-source");
    fs::write(&catalog_path, serde_json::to_vec_pretty(&catalog).unwrap()).unwrap();

    let output = workspace.path().join("invalid-package");
    let error = build_package(&fixture.join("profile.yml"), &output)
        .unwrap_err()
        .to_string();
    assert!(error.to_lowercase().contains("source"));
    assert!(!output.exists());
}
