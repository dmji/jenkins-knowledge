# Demo Environment: configuration

The Compose demo has its own local configuration and disposable Jenkins controller. It does not reuse a production profile or credentials.

1. Review `demo/README.md` and the pinned Jenkins integration test.
2. Run `bash demo/up.sh` from the repository root.
3. Keep generated `demo/.env` private (mode 0600).
4. Use the demo reader/admin identities only in this disposable environment.
5. Keep the controller bound to loopback; do not expose it outside the host.

The reader exercises Quick/Standard read permissions. The administrator identity is used only for the opt-in Deep Script Console probe. These are test fixtures, not a recommendation to use administrator privileges for production collection.
