# Historical compatibility target: Jenkins 2.356 with the official JDK 17 image.
# The linux/amd64 manifest digest pins the base image, including its bundled plugins.
FROM --platform=linux/amd64 jenkins/jenkins:2.356-jdk17@sha256:211419c659bd04b2d065ead531aad35ea47471489dcf714ab2c8a30cb9870850

USER root
COPY plugins/ /usr/share/jenkins/ref/plugins/
USER jenkins
