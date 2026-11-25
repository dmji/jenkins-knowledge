use jenkins_runtime::{Runtime, SearchOptions, DEFAULT_SEARCH_LIMIT, MAX_SEARCH_RESULTS};
use serde_json::json;
use std::{env, process};

fn usage() {
    eprintln!("Usage: jenkins-runtime --package <dir|manifest.json> [--profile <name>] <search|list|get|examples|step> [arguments] [--kind <kind>] [--source <source-id>] [--plugin <id>] [--owner <name>] [--type <type>] [--confidence <level>] [--limit <1..100>]");
    eprintln!(
        "Commands: search <query>, list, get <name-or-id>, examples <name-or-id>, step <step-name>, coverage"
    );
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1).peekable();
    let mut package = None;
    let mut profile = None;
    let mut positional = Vec::new();
    let mut kind = None;
    let mut source = None;
    let mut plugin = None;
    let mut owner = None;
    let mut type_name = None;
    let mut confidence = None;
    let mut limit = DEFAULT_SEARCH_LIMIT;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--package" | "-p" => package = args.next(),
            "--profile" => profile = args.next(),
            "--kind" => kind = args.next(),
            "--source" => source = args.next(),
            "--plugin" => plugin = args.next(),
            "--owner" => owner = args.next(),
            "--type" => type_name = args.next(),
            "--confidence" => confidence = args.next(),
            "--limit" => limit = args.next().ok_or("missing value for --limit")?.parse()?,
            "-h" | "--help" => {
                usage();
                return Ok(());
            }
            _ => positional.push(arg),
        }
    }
    let package = package.ok_or("missing required --package")?;
    let command = positional.first().ok_or("missing command")?;
    let query = positional.get(1).map(String::as_str).unwrap_or("");
    let runtime = Runtime::open(package)?;
    if let Some(requested) = profile {
        if requested != runtime.profile() {
            return Err(format!(
                "requested profile '{requested}' does not match package profile '{}'; select the matching package explicitly",
                runtime.profile()
            )
            .into());
        }
    }
    if !(1..=MAX_SEARCH_RESULTS).contains(&limit) {
        return Err(format!("--limit must be between 1 and {MAX_SEARCH_RESULTS}").into());
    }
    let search_options = SearchOptions {
        query: if command == "search" { query.to_owned() } else { String::new() },
        kind, source, plugin, owner, type_name, confidence, limit,
    };
    let output = match command.as_str() {
        "search" | "list" => json!({
            "profile": runtime.profile(),
            "limit": search_options.limit,
            "results": runtime.search_with_options(&search_options)
        }),
        "coverage" => json!({"profile": runtime.profile(), "coverage": runtime.coverage()}),
        "get" => runtime.get(query)?,
        "examples" => runtime.examples(query)?,
        "step" => runtime.pipeline_step(query)?,
        _ => {
            usage();
            return Err(format!("unknown command '{command}'").into());
        }
    };
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}
