# Configure Shared Libraries

Add each library to the profile's `libraries` array:

```yaml
libraries:
  - name: platform-lib
    url: https://git.example.com/platform/platform-lib.git
    revision: release/2.x
    commit: 0123456789abcdef0123456789abcdef01234567
```

The SHA is illustrative; replace it with the actual full 40-character commit SHA. Pinning makes collection reproducible. If omitted, Builder resolves the configured ref and records the resolved commit. Supply Git access through the credential helper or SSH agent; do not embed credentials in remotes.

Quick intentionally skips Shared Libraries. Standard and Deep attempt configured libraries. A failed library is isolated and recorded as a partial outcome. The scanner is a bounded declaration scanner, not a Groovy compiler: dynamic metaprogramming and unsupported syntax may not be discovered.
