const fs = require('fs');
const os = require('os');
const path = require('path');
const { runTests } = require('@vscode/test-electron');

async function main() {
    const required = [
        'JENKINS_LSP_PATH',
        'JENKINS_KNOWLEDGE_PRIMARY_PACKAGE',
        'JENKINS_KNOWLEDGE_ALTERNATE_PACKAGE'
    ];

    for (const name of required) {
        const value = process.env[name];
        if (!value || !fs.existsSync(value)) {
            throw new Error(`Missing or invalid ${name}: ${value || '<unset>'}`);
        }
    }

    const livePackage = process.env.JENKINS_KNOWLEDGE_LIVE_PACKAGE;
    if (livePackage && !fs.existsSync(livePackage)) {
        throw new Error(`Missing or invalid JENKINS_KNOWLEDGE_LIVE_PACKAGE: ${livePackage}`);
    }

    const extensionDevelopmentPath = path.resolve(__dirname, '..');
    const extensionTestsPath = path.join(extensionDevelopmentPath, 'out', 'test', 'suite', 'index.js');
    const workspace = fs.mkdtempSync(path.join(os.tmpdir(), 'jenkins-knowledge-vscode-'));

    try {
        await runTests({
            version: process.env.VSCODE_TEST_VERSION || '1.91.0',
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
    }
}

main().catch(error => {
    console.error(error);
    process.exit(1);
});
