# Jenkins 2.356 live integration target

This is a historical compatibility test, not a production deployment recommendation. It starts an isolated Jenkins controller and exercises the real Builder HTTP/Git collection path; fixture-only tests remain the fast default CI path.

## Pinned environment

| Component | Pin |
|---|---|
| Jenkins | Official `jenkins/jenkins:2.356-jdk17` image, pinned to linux/amd64 digest `sha256:211419c659bd04b2d065ead531aad35ea47471489dcf714ab2c8a30cb9870850` |
| Java | JDK 17 from the pinned Jenkins image |
| Authorization | Matrix Authorization Strategy `2.6.11`; SHA-256 `f70d8ba943478fb4966375e7c4ab213e7a0665bd88e9558d9c12492f1dee7dbd`; minimum Jenkins core `2.222.1` |
| Shared Library success fixture | `GSA/jenkins-shared-library-examples` at commit `7dfe2ab4bf7123b6aea29b0ec92caa2277cea5de` |
| Shared Library failure fixture | Same repository with an intentionally nonexistent, format-valid commit SHA |
| Rust | `1.100.0`, matching the repository validation toolchain |

The image digest locks the bundled Jenkins plugin set. The integration run captures the actual `/pluginManager/api/json` inventory and the Builder-produced controller inventory as artifacts, so the exact versions exercised are recorded rather than inferred from plugin names. The test additionally requires the Pipeline/GDSL and matrix authorization plugins used by this scenario.

Jenkins 2.356 is a weekly release. [Jenkins' Java support policy](https://www.jenkins.io/doc/book/platform-information/support-policy-java/) lists Java 17 support from weekly 2.340 through weekly 2.544; 2.356 is inside that support window. The test verifies the actual JVM version from the Script Console probe; it does not infer Java from the Core version. The [Matrix Authorization Strategy 2.6.11 release](https://updates.jenkins.io/download/plugins/matrix-auth/2.6.11/) requires Jenkins 2.222.1 and is compatible with this Core target.

## Permission matrix

The init script creates ephemeral users and API tokens:

| Profile | Account | Granted permissions | Expected Script Console behavior |
|---|---|---|---|
| Quick | `reader` | Overall/Read, Item/Read, View/Read | Skipped by profile |
| Standard | `reader` | Overall/Read, Item/Read, View/Read | Skipped by profile |
| Deep | `admin` | Overall/Administer | Executes and reports JVM metadata |
| Deep negative control | `reader` | Overall/Read, Item/Read, View/Read | Permission denied; other successful surfaces remain available |

Passwords and API tokens are generated for each run, stay inside the ephemeral container/runner, and are not written into the package. The container is removed when the script exits.

## Run locally or in CI

Requirements: Docker, Rust 1.100.0, Git, Python 3, curl, and OpenSSL.

```sh
bash tests/jenkins-2.356/run.sh
```

The script builds the pinned image, waits for Jenkins, runs Quick/Standard/Deep against the live controller, and writes evidence under `target/jenkins-2.356/`. It checks Core version, profile isolation, Script Console permission boundaries, Java metadata, plugin inventory, and both successful and failed pinned Shared Library revisions.

GitHub Actions runs this target as the `Jenkins 2.356 integration` job and uploads the sanitized package catalogs/manifests, Java version, image reference, and plugin inventory as `jenkins-2.356-integration-evidence`.

The target binds Jenkins to loopback only. Do not expose this old Jenkins image to an untrusted network or reuse its ephemeral accounts outside the test.
