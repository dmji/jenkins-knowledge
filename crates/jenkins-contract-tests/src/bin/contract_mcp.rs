#[cfg(not(test))]
#[allow(dead_code)]
#[path = "../../../jenkins-runtime/src/bin/mcp.rs"]
mod mcp;

#[cfg(not(test))]
fn main() {
    let mut args = std::env::args().skip(1);
    let mut package = None;
    while let Some(argument) = args.next() {
        if argument == "--package" || argument == "-p" {
            package = args.next();
        }
    }
    let Some(package) = package else {
        eprintln!("Usage: contract-mcp --package <knowledge-package-dir>");
        std::process::exit(2);
    };
    if let Err(error) = mcp::serve(&package) {
        eprintln!("contract-mcp: {error}");
        std::process::exit(2);
    }
}

#[cfg(test)]
fn main() {}
