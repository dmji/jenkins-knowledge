const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { runTests } = require('@vscode/test-electron');

async function main() {
    const root = path.resolve(__dirname, '../../..');
    const livePackage = path.resolve(process.env.JENKINS_KNOWLEDGE_LIVE_PACKAGE || '');
    const fixturePackage = path.resolve(process.env.JENKINS_KNOWLEDGE_FIXTURE_PACKAGE || '');
    const serverPath = path.resolve(process.env.JENKINS_KNOWLEDGE_LSP || path.join(root, 'target/release/jenkins-lsp'));
    for (const [name, value] of [['live package', livePackage], ['fixture package', fixturePackage], ['LSP server', serverPath]]) {
        if (!fs.existsSync(value)) throw new Error(`${name} does not exist: ${value}`);
    }

    const workspace = fs.mkdtempSync(path.join(os.tmpdir(), 'jenkins-knowledge-vscode-'));
    fs.mkdirSync(path.join(workspace, '.vscode'));
    fs.writeFileSync(path.join(workspace, '.vscode/settings.json'), JSON.stringify({
        'files.associations': { 'Jenkinsfile': 'groovy' },
        'jenkinsKnowledge.serverPath': serverPath,
        'jenkinsKnowledge.packagePath': livePackage,
        'jenkinsKnowledge.profile': 'jenkins-2356-deep'
    }, null, 2));
    fs.writeFileSync(path.join(workspace, 'Jenkinsfile'), [
        'node {',
        "  sh 'echo ok'",
        '  unavailableStep()',
        '  deployApp()',
        '  alternateOnly()',
        '  // unavailableStep()',
        '  def message = "unavailableStep()"',
        '  unknownStep()',
        '}',
        ''
    ].join('\n'));
    fs.writeFileSync(path.join(workspace, 'ordinary.groovy'), "node { sh 'echo ok' }");

    try {
        await runTests({
            version: '1.85.0',
            extensionDevelopmentPath: path.resolve(__dirname, '..'),
            extensionTestsPath: path.join(__dirname, 'suite/index.js'),
            launchArgs: [workspace, '--disable-workspace-trust', '--skip-welcome', '--skip-release-notes'],
            extensionTestsEnv: {
                JENKINS_KNOWLEDGE_FIXTURE_PACKAGE: fixturePackage,
                JENKINS_KNOWLEDGE_ALTERNATE_PACKAGE: process.env.JENKINS_KNOWLEDGE_ALTERNATE_PACKAGE || ''
            }
        });
    } finally {
        fs.rmSync(workspace, { recursive: true, force: true });
    }
}

main().catch(error => {
    console.error(error);
    process.exitCode = 1;
});
