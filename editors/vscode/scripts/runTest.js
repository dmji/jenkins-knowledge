const fs = require('fs');
const os = require('os');
const path = require('path');
const { runTests } = require('@vscode/test-electron');

async function main() {
    const liveOnly = process.env.JENKINS_KNOWLEDGE_LIVE_ONLY === 'true';
    const required = ['JENKINS_LSP_PATH'];
    if (!liveOnly) {
        required.push(
            'JENKINS_KNOWLEDGE_PRIMARY_PACKAGE',
            'JENKINS_KNOWLEDGE_ALTERNATE_PACKAGE'
        );
    }

    for (const name of required) {
        const value = process.env[name];
        if (!value || !fs.existsSync(value)) {
            throw new Error(`Missing or invalid ${name}: ${value || '<unset>'}`);
        }
    }

    const livePackage = process.env.JENKINS_KNOWLEDGE_LIVE_PACKAGE;
    if (livePackage && process.env.JENKINS_KNOWLEDGE_LIVE_PACKAGE_REMOTE !== 'true' && !fs.existsSync(livePackage)) {
        throw new Error(`Missing or invalid JENKINS_KNOWLEDGE_LIVE_PACKAGE: ${livePackage}`);
    }

    const extensionDevelopmentPath = path.resolve(__dirname, '..');
    const extensionTestsPath = path.join(extensionDevelopmentPath, 'out', 'test', 'suite', 'index.js');
    const workspace = fs.mkdtempSync(path.join(os.tmpdir(), 'jenkins-knowledge-vscode-'));

    try {
        const extensionTestsEnv = {};
        for (const name of [
            'JENKINS_LSP_PATH',
            'JENKINS_KNOWLEDGE_LIVE_PACKAGE',
            'JENKINS_KNOWLEDGE_LIVE_PROFILE',
            'JENKINS_KNOWLEDGE_PRIMARY_PACKAGE',
            'JENKINS_KNOWLEDGE_ALTERNATE_PACKAGE',
            'JENKINS_KNOWLEDGE_LIVE_ONLY'
        ]) {
            if (process.env[name]) extensionTestsEnv[name] = process.env[name];
        }

        await runTests({
            version: process.env.VSCODE_TEST_VERSION || '1.91.0',
            extensionDevelopmentPath,
            extensionTestsPath,
            extensionTestsEnv,
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
