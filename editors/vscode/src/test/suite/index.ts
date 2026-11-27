import * as path from 'path';
import * as vscode from 'vscode';
import * as assert from 'assert';

export function run(): void {
    suite('Jenkins Knowledge VS Code extension', () => {
        const workspace = process.env.JENKINS_KNOWLEDGE_TEST_WORKSPACE!;
        const jenkinsfile = vscode.Uri.file(path.join(workspace, 'Jenkinsfile'));
        const ordinaryGroovy = vscode.Uri.file(path.join(workspace, 'ordinary.groovy'));

        test('activates and provides catalog completion and hover for Jenkinsfiles', async () => {
            const document = await vscode.workspace.openTextDocument(jenkinsfile);
            await vscode.window.showTextDocument(document);
            const completions = await waitForCompletion(jenkinsfile, 'deployApp');
            assert.ok(completions.items.some(item => item.label === 'deployApp'));

            const hovers = await vscode.commands.executeCommand<vscode.Hover[]>(
                'vscode.executeHoverProvider',
                jenkinsfile,
                new vscode.Position(1, 4)
            );
            assert.ok(hovers?.some(hover => hover.contents.some(content =>
                (typeof content === 'string' ? content : content.value).includes('deployApp')
            )));
            const definitions = await vscode.commands.executeCommand<Array<vscode.Location | vscode.LocationLink>>(
                'vscode.executeDefinitionProvider',
                jenkinsfile,
                new vscode.Position(1, 4)
            );
            const target = definitions?.[0] instanceof vscode.Location
                ? definitions[0].uri
                : definitions?.[0]?.targetUri;
            assert.ok(target?.toString().includes('github.com/example/fixture-library/blob/'));

            await waitForDiagnostics(jenkinsfile);
        });

        test('does not provide Jenkins catalog completion for ordinary Groovy files', async () => {
            const document = await vscode.workspace.openTextDocument(ordinaryGroovy);
            await vscode.window.showTextDocument(document);
            const completions = await vscode.commands.executeCommand<vscode.CompletionList>(
                'vscode.executeCompletionItemProvider',
                ordinaryGroovy,
                new vscode.Position(0, 5)
            );
            assert.ok(!completions?.items.some(item => item.label === 'deployApp'));
        });

        test('clears catalog results after a profile mismatch and restores them on reload', async () => {
            const configuration = vscode.workspace.getConfiguration('jenkinsKnowledge');
            await configuration.update('profile', 'wrong-profile', vscode.ConfigurationTarget.Workspace);
            await vscode.commands.executeCommand('jenkinsKnowledge.reload');
            await new Promise(resolve => setTimeout(resolve, 150));
            const completions = await vscode.commands.executeCommand<vscode.CompletionList>(
                'vscode.executeCompletionItemProvider', jenkinsfile, new vscode.Position(1, 6)
            );
            assert.ok(!completions?.items.some(item => item.label === 'deployApp'));

            await configuration.update('profile', 'contract-primary', vscode.ConfigurationTarget.Workspace);
            await vscode.commands.executeCommand('jenkinsKnowledge.reload');
            const restored = await waitForCompletion(jenkinsfile, 'deployApp');
            assert.ok(restored.items.some(item => item.label === 'deployApp'));
        });
    });
}

async function waitForCompletion(uri: vscode.Uri, label: string): Promise<vscode.CompletionList> {
    for (let attempt = 0; attempt < 40; attempt++) {
        const result = await vscode.commands.executeCommand<vscode.CompletionList>(
            'vscode.executeCompletionItemProvider', uri, new vscode.Position(1, 6)
        );
        if (result?.items.some(item => item.label === label)) return result;
        await new Promise(resolve => setTimeout(resolve, 250));
    }
    throw new Error(`Timed out waiting for ${label} completion`);
}

async function waitForDiagnostics(uri: vscode.Uri): Promise<void> {
    for (let attempt = 0; attempt < 40; attempt++) {
        if (vscode.languages.getDiagnostics(uri).some(diagnostic =>
            diagnostic.code === 'jenkins.plugin.unavailable'
        )) return;
        await new Promise(resolve => setTimeout(resolve, 250));
    }
    throw new Error('Timed out waiting for unavailable-plugin diagnostics');
}
