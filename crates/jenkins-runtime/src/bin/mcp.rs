use jenkins_runtime::Runtime;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

fn required<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
        .ok_or_else(|| format!("'{key}' must be a non-empty string"))
}
fn optional<'a>(args: &'a Value, key: &str) -> Option<&'a str> { args.get(key).and_then(Value::as_str) }
fn content(value: Value) -> Value {
    json!({"content":[{"type":"text","text":serde_json::to_string_pretty(&value).unwrap_or_else(|e|e.to_string())}]})
}
fn tools() -> Value {
    json!([
      {"name":"search_api","description":"Search catalog symbols. Results include profile, confidence, and source provenance.","inputSchema":{"type":"object","properties":{"query":{"type":"string"},"kind":{"type":"string"},"source":{"type":"string"}},"additionalProperties":false}},
      {"name":"get_api_details","description":"Get one symbol by exact name or ID, including signature, parameters, provenance, profile, and confidence.","inputSchema":{"type":"object","properties":{"name_or_id":{"type":"string"}},"required":["name_or_id"],"additionalProperties":false}},
      {"name":"list_api","description":"List catalog symbols, optionally filtered by kind and source.","inputSchema":{"type":"object","properties":{"kind":{"type":"string"},"source":{"type":"string"}},"additionalProperties":false}},
      {"name":"get_usage_examples","description":"Get catalog-backed usage examples for one exact symbol name or ID.","inputSchema":{"type":"object","properties":{"name_or_id":{"type":"string"}},"required":["name_or_id"],"additionalProperties":false}},
      {"name":"get_pipeline_step","description":"Get Jenkins Pipeline step metadata by exact name or symbol ID.","inputSchema":{"type":"object","properties":{"name":{"type":"string"}},"required":["name"],"additionalProperties":false}}
    ])
}
fn call_tool(rt: &Runtime, name: &str, args: &Value) -> Result<Value,String> {
    match name {
        "search_api" => Ok(content(json!({"profile":rt.profile(),"results":rt.search(optional(args,"query").unwrap_or(""),optional(args,"kind"),optional(args,"source"))}))),
        "list_api" => Ok(content(json!({"profile":rt.profile(),"results":rt.search("",optional(args,"kind"),optional(args,"source"))}))),
        "get_api_details" => rt.get(required(args,"name_or_id")?).map(content).map_err(|e|e.to_string()),
        "get_usage_examples" => rt.examples(required(args,"name_or_id")?).map(content).map_err(|e|e.to_string()),
        "get_pipeline_step" => rt.pipeline_step(required(args,"name")?).map(content).map_err(|e|e.to_string()),
        _ => Err(format!("unknown tool '{name}'")),
    }
}
fn read_resource(rt: &Runtime, uri: &str) -> Result<Value,String> {
    match uri {
        "jenkins://catalog/manifest" => Ok(rt.manifest().clone()),
        "jenkins://catalog/libraries" => Ok(json!({"profile":rt.profile(),"sources":rt.sources().iter().filter(|s|s["kind"]=="shared-library").collect::<Vec<_>>()})),
        "jenkins://catalog/pipeline-steps" => Ok(json!({"profile":rt.profile(),"steps":rt.search("",Some("step"),None)})),
        _ if uri.starts_with("jenkins://catalog/profiles/") => {
            let profile=uri.trim_start_matches("jenkins://catalog/profiles/");
            if profile!=rt.profile() { return Err(format!("profile '{profile}' is not loaded; launch the server with that profile's package")); }
            Ok(json!({"profile":rt.profile(),"manifest":rt.manifest(),"catalogId":rt.catalog()["catalogId"],"schemaVersion":rt.catalog()["schemaVersion"]}))
        }
        _ => Err(format!("unknown resource '{uri}'")),
    }
}
fn reply(id: Value, result: Value) -> Value { json!({"jsonrpc":"2.0","id":id,"result":result}) }
fn handle(rt: &Runtime, req: Value) -> Option<Value> {
    let id=req.get("id").cloned()?;
    let method=req.get("method").and_then(Value::as_str).unwrap_or("");
    let params=req.get("params").cloned().unwrap_or_else(||json!({}));
    let result: Result<Value,Value> = match method {
        "initialize" => Ok(json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{},"resources":{"subscribe":false,"listChanged":false}},"serverInfo":{"name":"jenkins-knowledge","version":env!("CARGO_PKG_VERSION")}})),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools":tools()})),
        "tools/call" => {
            let name=params.get("name").and_then(Value::as_str).unwrap_or("");
            let args=params.get("arguments").cloned().unwrap_or_else(||json!({}));
            match call_tool(rt,name,&args) { Ok(v)=>Ok(v), Err(e)=>Ok(json!({"content":[{"type":"text","text":e}],"isError":true})) }
        }
        "resources/list" => Ok(json!({"resources":[
            {"uri":"jenkins://catalog/manifest","name":"Knowledge package manifest","mimeType":"application/json"},
            {"uri":"jenkins://catalog/libraries","name":"Shared Library sources","mimeType":"application/json"},
            {"uri":"jenkins://catalog/pipeline-steps","name":"Jenkins Pipeline steps","mimeType":"application/json"},
            {"uri":format!("jenkins://catalog/profiles/{}",rt.profile()),"name":"Selected profile","mimeType":"application/json"}
        ]})),
        "resources/read" => {
            let uri=params.get("uri").and_then(Value::as_str).unwrap_or("");
            match read_resource(rt,uri) {
                Ok(v)=>Ok(json!({"contents":[{"uri":uri,"mimeType":"application/json","text":serde_json::to_string_pretty(&v).unwrap_or_default()}]})),
                Err(e)=>Err(json!({"code":-32602,"message":e}))
            }
        }
        _ => Err(json!({"code":-32601,"message":format!("method not found: {method}")})),
    };
    Some(match result { Ok(v)=>reply(id,v), Err(e)=>json!({"jsonrpc":"2.0","id":id,"error":e}) })
}
fn main() {
    let mut args=std::env::args().skip(1); let mut package=None;
    while let Some(arg)=args.next() { if arg=="--package" || arg=="-p" { package=args.next(); } }
    let package=package.or_else(||std::env::var("JENKINS_KNOWLEDGE_PACKAGE").ok()).unwrap_or_else(||{
        eprintln!("Usage: jenkins-mcp --package <knowledge-package-dir>"); std::process::exit(2)
    });
    let runtime=Runtime::open(package).unwrap_or_else(|e|{eprintln!("Cannot load Knowledge Package: {e}");std::process::exit(2)});
    let stdin=io::stdin(); let mut stdout=io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let Ok(line)=line else { break }; if line.trim().is_empty(){continue;}
        let response=match serde_json::from_str::<Value>(&line) {
            Ok(req)=>handle(&runtime,req),
            Err(e)=>Some(json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":e.to_string()}}))
        };
        if let Some(response)=response { if writeln!(stdout,"{response}").and_then(|_|stdout.flush()).is_err(){break;} }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_public_tool_has_an_object_input_schema() {
        let definitions = tools();
        let definitions = definitions.as_array().unwrap();
        let names: Vec<_> = definitions.iter().filter_map(|tool| tool["name"].as_str()).collect();
        assert_eq!(names, ["search_api", "get_api_details", "list_api", "get_usage_examples", "get_pipeline_step"]);
        for tool in definitions {
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert!(tool["inputSchema"]["properties"].is_object());
        }
    }

    #[test]
    fn detail_tools_require_their_lookup_argument() {
        let definitions = tools();
        for (name, argument) in [
            ("get_api_details", "name_or_id"),
            ("get_usage_examples", "name_or_id"),
            ("get_pipeline_step", "name"),
        ] {
            let tool = definitions.as_array().unwrap().iter().find(|tool| tool["name"] == name).unwrap();
            assert!(tool["inputSchema"]["required"].as_array().unwrap().iter().any(|v| v == argument));
        }
    }
}
