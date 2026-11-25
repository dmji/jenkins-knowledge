use url::Url;

/// Jenkins controller endpoints must be plain HTTP(S) URLs without embedded credentials
/// or request modifiers. Credentials are supplied separately at runtime.
pub fn validate_controller_url(value: &str) -> Result<(), String> {
    let parsed = Url::parse(value).map_err(|_| "must be an absolute HTTP(S) URL".to_owned())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("only absolute HTTP(S) URLs with a host are supported".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("credentials must be supplied through the runtime environment, not the URL".into());
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("query parameters and fragments are not allowed".into());
    }
    Ok(())
}

/// Allow HTTPS and SSH URLs, plus Git's SCP-like syntax. Local paths, git://,
/// file://, credential-bearing URLs, query strings, and fragments are rejected.
pub fn validate_git_url(value: &str) -> Result<(), String> {
    shared_library_scanner::validate_git_url(value)
}


/// Remove user information from a URL before it is copied into package metadata.
pub fn redact_url(value: &str) -> String {
    if let Ok(mut parsed) = Url::parse(value) {
        let _ = parsed.set_username("");
        let _ = parsed.set_password(None);
        parsed.set_query(None);
        parsed.set_fragment(None);
        return parsed.to_string();
    }
    let without_suffix = value.split(['?', '#']).next().unwrap_or(value);
    without_suffix.rsplit_once('@').map_or_else(|| without_suffix.to_owned(), |(_, rest)| rest.to_owned())
}

pub fn validate_git_reference(value: &str) -> Result<(), String> {
    shared_library_scanner::validate_git_reference(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_urls_require_http_without_userinfo_or_query() {
        assert!(validate_controller_url("https://jenkins.example.test/").is_ok());
        for value in [
            "file:///etc/passwd",
            "https://user:secret@jenkins.example.test/",
            "https://jenkins.example.test/?token=secret",
            "https://jenkins.example.test/#fragment",
        ] {
            assert!(validate_controller_url(value).is_err(), "accepted {value}");
        }
    }

    #[test]
    fn git_urls_are_restricted_to_safe_transports() {
        for value in [
            "https://github.example.test/team/library.git",
            "ssh://git@github.example.test/team/library.git",
            "git@github.example.test:team/library.git",
        ] {
            assert!(validate_git_url(value).is_ok(), "rejected {value}");
        }
        for value in [
            "git://github.example.test/team/library.git",
            "file:///tmp/library",
            "/tmp/library",
            "https://user:secret@github.example.test/team/library.git",
            "ssh://git:secret@github.example.test/team/library.git",
            "git:secret@github.example.test:team/library.git",
            "https://github.example.test/team/library.git?token=secret",
            "-upload-pack=evil",
        ] {
            assert!(validate_git_url(value).is_err(), "accepted {value}");
        }
    }

    #[test]
    fn metadata_urls_do_not_retain_user_information() {
        assert_eq!(
            redact_url("ssh://git:secret@github.example.test/team/library.git"),
            "ssh://github.example.test/team/library.git"
        );
        assert_eq!(
            redact_url("git@github.example.test:team/library.git"),
            "github.example.test:team/library.git"
        );
        assert_eq!(
            redact_url("https://github.example.test/team/library.git?token=secret#fragment"),
            "https://github.example.test/team/library.git"
        );
    }

    #[test]
    fn git_references_reject_option_injection_and_malformed_names() {
        assert!(validate_git_reference("release/2.x").is_ok());
        assert!(validate_git_reference("0123456789abcdef0123456789abcdef01234567").is_ok());
        for value in ["", "-c", "main branch", "bad..ref", "topic@{1}", "trailing/", ".hidden/main", "main/.hidden", "main.lock/sub"] {
            assert!(validate_git_reference(value).is_err(), "accepted {value}");
        }
    }
}
