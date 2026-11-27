# Choose a profile and run collection

`collectionProfile` is separate from the package's `profile` identity.

| Profile | Behavior |
| --- | --- |
| `quick` (default) | Required Core/controller metadata; optional plugin inventory; GDSL availability probe only. Skips full GDSL, libraries, and Script Console. |
| `standard` | Core metadata plus optional plugin inventory, full GDSL extraction, and configured Shared Libraries. No Script Console. |
| `deep` | Standard plus fixed bounded read-only Script Console probe and explicitly configured API roots. |

Java version/vendor are read from actual JVM properties only by the Deep probe; never infer them from Core version. Deep requires explicit approval.

```yaml
profile: production-pipelines
collectionProfile: standard
jenkins:
  controllers:
    - id: production
      url: https://jenkins.example.com
libraries:
  - name: platform-lib
    url: https://git.example.com/platform/platform-lib.git
    revision: release/2.x
    commit: 0123456789abcdef0123456789abcdef01234567
package:
  id: production-pipelines
```

Replace the illustrative SHA or omit `commit`. YAML is strict; unknown keys are errors. One controller per package is supported.

```sh
cargo run --manifest-path crates/knowledge-builder/Cargo.toml -- plan profiles/production-pipelines.yml
cargo run --manifest-path crates/knowledge-builder/Cargo.toml -- build profiles/production-pipelines.yml ./out/production-pipelines-2026-11
```

The output path must not exist. Retries are explicit. Only fully successful controller snapshots are cached locally; cache data is not part of the package.
