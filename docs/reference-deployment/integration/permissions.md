# Jenkins service account and permissions

Use a dedicated service identity, not a personal administrator account, for routine collection. The live integration test verifies Quick and Standard using a read-oriented identity granted `Overall/Read`, `Item/Read`, and `View/Read`. Actual requirements may vary with Jenkins security configuration and plugins; begin with least privilege and inspect source outcomes.

Do not grant broader permissions just to make optional inventory complete. Plugin inventory may remain unavailable/unknown and should be represented honestly.

## Deep profile

Deep invokes a fixed, repository-owned, bounded, read-only Script Console probe. The live test runs it with `Overall/Administer`. Do not grant this to routine service identities. Deep requires explicit security approval. A read-only Deep negative control is tested; Script Console denial remains an optional-source outcome and is not bypassed.

For Core metadata failures, check reachability, authentication, reverse-proxy rules, and read access. For GDSL/inventory failures, inspect the corresponding endpoint and per-surface outcome before changing grants.
