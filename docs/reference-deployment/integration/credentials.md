# Supply credentials securely

Builder reads Jenkins credentials from `JENKINS_USERNAME` and `JENKINS_API_TOKEN`. Provide both or neither. Prefer a dedicated Jenkins user and API token; omit both only when the controller permits the required read requests anonymously.

For an interactive shell:

```sh
read -r -p "Jenkins username: " JENKINS_USERNAME
export JENKINS_USERNAME
read -r -s -p "Jenkins API token: " JENKINS_API_TOKEN
printf '\\n'
export JENKINS_API_TOKEN
knowledge-builder build profiles/production-pipelines.yml ./out/production-pipelines-2026-11
unset JENKINS_USERNAME JENKINS_API_TOKEN
```

For automation, inject secrets from the CI secret store at job runtime. Never commit tokens, place them in profile URLs/command arguments/logs, bake them into images, or copy them into packages. Git credentials come from the Builder host's normal Git credential helper or SSH agent.
