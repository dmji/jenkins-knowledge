# Historical compatibility target: Jenkins 2.356 with the official JDK 17 image.
# The linux/amd64 manifest digest pins the complete base image and its bundled plugins.
FROM --platform=linux/amd64 jenkins/jenkins:2.356-jdk17@sha256:211419c659bd04b2d065ead531aad35ea47471489dcf714ab2c8a30cb9870850

USER root
COPY plugins.txt /usr/share/jenkins/ref/jenkins-knowledge-plugins.txt
RUN jenkins-plugin-cli \
      --plugin-file /usr/share/jenkins/ref/jenkins-knowledge-plugins.txt \
      --plugin-download-directory /usr/share/jenkins/ref/plugins \
    && plugin="$(find /usr/share/jenkins/ref/plugins -maxdepth 1 -type f -name 'matrix-auth.*' -print -quit)" \
    && test -n "$plugin" \
    && printf '%s  %s\\n' 'f70d8ba943478fb4966375e7c4ab213e7a0665bd88e9558d9c12492f1dee7dbd' "$plugin" | sha256sum -c -
USER jenkins
