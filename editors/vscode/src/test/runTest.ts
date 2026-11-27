import * as os from 'os';
import * as path from 'path';
import { mkdtempSync, mkdirSync, writeFileSync } from 'fs';
import { runTests } from '@vscode/test-electron';

async function main(): Promise<void> {
    const extensionDevelopmentPath = path.resolve(__dirname, '../..');
    const workspace = mkdtempSync(path.join(os.tmpdir(), 'jenkins-knowledge-vscode-'));
    mkdirSync(path.join(workspace, '.vscode'));
    writeFileSync(path.join(workspace, 'Jenkinsfile'), "node {\n  deployApp()\n  unavailableStep()\n}\n");
    writeFileSync(path.join(workspace, 'ordinary.groovy'), 'def deploy() {}\n');
    writeFileSync(path.join(workspace, '.vscode', 'settings.json'), JSON.stringify({
        'jenkinsKnowledge.serverPath': process.env.JENKINS_LSP_PATH,
        'jenkinsKnowledge.packagePath': process.env.JENKINS_KNOWLEDGE_PACKAGE,
        'jenkinsKnowledge.profile': 'contract-primary',
        'files.associations': { 'Jenkinsfile': 'groovy' }
    }, null, 2));

    await runTests({
        extensionDevelopmentPath,
        extensionTestsPath: path.resolve(__dirname, './suite/index'),
        launchArgs: [workspace, '--disable-gpu', '--no-sandbox', '--disable-workspace-trust'],
        extensionTestsEnv: { JENKINS_KNOWLEDGE_TEST_WORKSPACE: workspace }
    });
}

main().catch(error => {
    console.error(error);
    process.exit(1);
});
