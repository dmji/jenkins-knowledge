# Connect an existing controller

The Builder profile identifies the controller URL. Start from `profiles/production-pipelines.yml` and replace the example host:

```yaml
profile: production-pipelines
collectionProfile: standard
jenkins:
  controllers:
    - id: production
      url: https://jenkins.example.com
libraries: []
package:
  id: production-pipelines
```

Use the real HTTPS origin/context path. Never embed username, password, API token, query parameters, or fragments in the URL; the profile parser rejects URL user information, query parameters, and fragments.

Inspect the network-free plan before collection:

```sh
cargo run --manifest-path crates/knowledge-builder/Cargo.toml -- plan profiles/production-pipelines.yml
```

It reports endpoints, request budgets/limits, Script Console probe details, and Git remotes without reading secrets or contacting Jenkins/Git. Core/controller metadata is mandatory; optional plugin/GDSL failures remain explicit outcomes and can make the package partial.

Only one controller per package is currently supported. Create separate profiles/packages for separate controllers.
