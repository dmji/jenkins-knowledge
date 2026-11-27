import hudson.model.Item
import hudson.model.User
import hudson.model.View
import hudson.security.GlobalMatrixAuthorizationStrategy
import hudson.security.HudsonPrivateSecurityRealm
import jenkins.model.Jenkins
import jenkins.security.ApiTokenProperty

def jenkins = Jenkins.get()
def matrixAuth = jenkins.pluginManager.getPlugin("matrix-auth")
if (matrixAuth == null || matrixAuth.version != "2.6.11") {
    throw new IllegalStateException("Expected matrix-auth 2.6.11, found: " + (matrixAuth == null ? "missing" : matrixAuth.version))
}

def adminPassword = System.getenv("TEST_ADMIN_PASSWORD")
def readerPassword = System.getenv("TEST_READER_PASSWORD")
if (!adminPassword || !readerPassword) {
    throw new IllegalStateException("Test account passwords were not provided")
}

def realm = new HudsonPrivateSecurityRealm(false)
jenkins.setSecurityRealm(realm)
def admin = realm.createAccount("admin", adminPassword)
def reader = realm.createAccount("reader", readerPassword)

def createToken = { User user, String name ->
    def property = user.getProperty(ApiTokenProperty.class)
    if (property == null) {
        property = new ApiTokenProperty()
        user.addProperty(property)
    }
    property.tokenStore.generateNewToken(name).plainValue
}

def adminToken = createToken(admin, "jenkins-knowledge-integration-admin")
def readerToken = createToken(reader, "jenkins-knowledge-integration-reader")

def authorization = new GlobalMatrixAuthorizationStrategy()
authorization.add(Jenkins.ADMINISTER, "admin")
authorization.add(Jenkins.READ, "reader")
authorization.add(Item.READ, "reader")
authorization.add(View.READ, "reader")
jenkins.setAuthorizationStrategy(authorization)
jenkins.save()

def tokenFilePath = System.getenv("JENKINS_KNOWLEDGE_TOKEN_FILE") ?: "/tmp/jenkins-knowledge-tokens.env"
def tokenFile = new File(tokenFilePath)
tokenFile.text = "ADMIN_TOKEN=" + adminToken + "\nREADER_TOKEN=" + readerToken + "\n"
tokenFile.setReadable(false, false)
tokenFile.setReadable(true, true)

if (System.getenv("JENKINS_KNOWLEDGE_DEMO") == "true") {
    def gitSource = new jenkins.plugins.git.GitSCMSource(
        "gsa-examples",
        "https://github.com/GSA/jenkins-shared-library-examples.git",
        null,
        "*",
        "",
        false
    )
    def library = new org.jenkinsci.plugins.workflow.libs.LibraryConfiguration(
        "gsa-examples",
        new org.jenkinsci.plugins.workflow.libs.SCMSourceRetriever(gitSource)
    )
    library.setDefaultVersion("main")
    library.setImplicit(false)
    library.setAllowVersionOverride(false)
    def globalLibraries = org.jenkinsci.plugins.workflow.libs.GlobalLibraries.get()
    globalLibraries.setLibraries([library])
    globalLibraries.save()

    def demoJobName = "jenkins-knowledge-demo"
    def demoJob = jenkins.getItem(demoJobName)
    if (demoJob == null) {
        demoJob = jenkins.createProject(org.jenkinsci.plugins.workflow.job.WorkflowJob.class, demoJobName)
    }
    def demoPipeline = '''@Library('gsa-examples') _
pipeline {
    agent any
    stages {
        stage('Controller') {
            steps {
                echo 'Jenkins Knowledge local demo'
                sh 'java -version'
            }
        }
        stage('Pinned Shared Library') {
            steps {
                example1('Jenkins Knowledge')
            }
        }
    }
}'''
    demoJob.setDefinition(new org.jenkinsci.plugins.workflow.cps.CpsFlowDefinition(demoPipeline, true))
    demoJob.save()
}
