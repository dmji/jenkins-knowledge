# Demo Environment: installation

This optional demo starts a disposable Jenkins 2.356 / Java 17 controller. It is not required to integrate Jenkins Knowledge with an existing controller and is not a production deployment recipe.

## Requirements

- Docker Engine and Docker Compose v2;
- Rust 1.100.0, Git, Python 3, curl, OpenSSL;
- Node.js 26.13.1/npm 11.20.0 and `xvfb-run` for VS Code host acceptance.

The image and plugin set are pinned in [the live integration test reference](../../tests/jenkins-2.356/README.md).

## Start and stop

From repository root:

```sh
bash demo/up.sh
bash demo/build-package.sh
bash demo/verify.sh
bash demo/down.sh
```

The demo binds Jenkins to `127.0.0.1:18080`. It is disposable; never expose it to an untrusted network. Demo secrets live in `demo/.env`; keep it mode 0600, never commit it, and never reuse its credentials. For exact variables and lifecycle details see [demo/README.md](../../../demo/README.md).
