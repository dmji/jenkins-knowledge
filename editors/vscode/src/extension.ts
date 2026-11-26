import * as path from 'path';
import * as vscode from 'vscode';
import { LanguageClient, LanguageClientOptions, ServerOptions } from 'vscode-languageclient/node';

let client: LanguageClient | undefined;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
    const folder = vscode.workspace.workspaceFolders?.[0];
    if (!folder) {
        return;
    }

    const configuration = vscode.workspace.getConfiguration('jenkinsKnowledge');
    const packagePath = configuration.get<string>('packagePath', '');
    if (!packagePath) {
        void vscode.window.showWarningMessage(
            'Set jenkinsKnowledge.packagePath to a built Knowledge Package to enable Jenkins catalog features.'
        );
    }

    const serverPath = configuration.get<string>('serverPath', 'jenkins-lsp');
    const serverOptions: ServerOptions = {
        command: serverPath,
        args: [],
        options: { cwd: folder.uri.fsPath }
    };
    const clientOptions: LanguageClientOptions = {
        documentSelector: [{ scheme: 'file', language: 'groovy' }],
        initializationOptions: {
            packagePath: packagePath ? path.resolve(folder.uri.fsPath, packagePath) : '',
            profile: configuration.get<string>('profile', ''),
            workspaceRoot: folder.uri.fsPath
        },
        synchronize: {
            configurationSection: 'jenkinsKnowledge'
        }
    };

    client = new LanguageClient('jenkinsKnowledge', 'Jenkins Knowledge', serverOptions, clientOptions);
    context.subscriptions.push(client);
    await client.start();

    context.subscriptions.push(vscode.commands.registerCommand('jenkinsKnowledge.reload', async () => {
        const current = vscode.workspace.getConfiguration('jenkinsKnowledge');
        const selectedPackage = current.get<string>('packagePath', '');
        if (!selectedPackage) {
            void vscode.window.showWarningMessage('Set jenkinsKnowledge.packagePath before reloading the Knowledge Package.');
            return;
        }
        try {
            const result = await client?.sendRequest<{ status: string; profile: string }>('jenkinsKnowledge/reload', {
                packagePath: path.resolve(folder.uri.fsPath, selectedPackage),
                profile: current.get<string>('profile', '')
            });
            if (result) void vscode.window.showInformationMessage(`Jenkins Knowledge loaded profile '${result.profile}'.`);
        } catch (error) {
            void vscode.window.showErrorMessage(`Jenkins Knowledge reload failed: ${String(error)}`);
        }
    }));
}

export async function deactivate(): Promise<void> {
    await client?.stop();
}
