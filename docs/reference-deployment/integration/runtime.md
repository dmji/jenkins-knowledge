# Configure Runtime CLI

Runtime is a local read-only consumer. It does not contact Jenkins/Git, use collection credentials, start Builder, or refresh package contents.

```sh
/opt/jenkins-knowledge/tools/bin/jenkins-runtime --package /srv/jenkins-knowledge/production-pipelines search deploy
/opt/jenkins-knowledge/tools/bin/jenkins-runtime --package /srv/jenkins-knowledge/production-pipelines list --kind step
/opt/jenkins-knowledge/tools/bin/jenkins-runtime --package /srv/jenkins-knowledge/production-pipelines get deployApp
/opt/jenkins-knowledge/tools/bin/jenkins-runtime --package /srv/jenkins-knowledge/production-pipelines examples deployApp
/opt/jenkins-knowledge/tools/bin/jenkins-runtime --package /srv/jenkins-knowledge/production-pipelines step sh
```

Use the release executable in production rather than invoking Cargo per query. Package is mandatory unless `JENKINS_KNOWLEDGE_PACKAGE` is set. Optional `--profile` must match the manifest. Mount the package read-only. Results include profile, source snapshot, confidence, and provenance.
