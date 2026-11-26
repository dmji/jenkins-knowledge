#[cfg(not(test))]
#[allow(dead_code)]
#[path = "../../../jenkins-lsp/src/main.rs"]
mod lsp;

#[cfg(not(test))]
fn main() {
    if let Err(error) = lsp::serve() {
        eprintln!("contract-lsp: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
fn main() {}
