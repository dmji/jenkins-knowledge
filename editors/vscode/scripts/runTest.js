const fs = require('fs');
const os = require('os');
const path = require('path');
const { runTests } = require('@vscode/test-electron');

const required = [
    'JENKINS_LSP_PATH',
    'JENKINS_KNOWLEDGE_LIVE_PACKAGE',
    'JENKINS_KNOWLEDGE_PRIMARY_PACKAGE',
    'JENKINS_KNOWLEDGE_ALTERNATE_PACKAGE'
];

for (const name of required) {
    const value = process.env[name];
    if (!value || !fs.existsSync(value)) {
        throw new Error(`Missing or invalid ${name}: ${value || '<unset>'}`);
    }
}

const extensionDevelopmentPath = path.resolve(__dirname, '..');
const extensionTestsPath = path.join(extensionDevelopmentPath, 'out', 'test', 'suite', 'index.js');
const workspace = fs.mkdtempSync(path.join(os.tmpdir(), 'jenkins-knowledge-vscode-'));

try {
    await runTests({
        version: process.env.VSCODE_TEST_VERSION || '1.85.0',
        extensionDevelopmentPath,
        extensionTestsPath,
        launchArgs: [
            workspace,
            '--disable-workspace-trust',
            '--disable-gpu',
            '--no-sandbox'
        ]
    });
} finally {
    fs.rmSync(workspace, { recursive: true, force: true });
}.catch(error => {
    console.error(error);
    process.exit(1);
});
