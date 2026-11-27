import * as assert from 'assert';
import * as path from 'path';
import * as vscode from 'vscode';

function workspacePath(): string {
    const folder = vscode.workspace.workspaceFolders?.[0];
    assert.ok(folder, 'VS Code test workspace was not opened.');
    return folder.uri.fsPath;
}

async function writeJenkinsfile(name: string, contents: string): Promise<vscode.TextDocument> {
    const uri = vscode.Uri.file(path.join(workspacePath(), name));
    await vscode.workspace.fs.writeFile(uri, Buffer.from(contents, 'utf8'));
    const document = await vscode.workspace.openTextDocument(uri);
    assert.strictEqual(document.languageId, 'groovy', `${name} must be associated with Groovy.`);
    await vscode.window.showTextDocument(document);
    return document;
}

async function completionLabels(
    document: vscode.TextDocument,
    position: vscode.Position
): Promise<string[]> {
    const result = await vscode.commands.executeCommand<
        vscode.CompletionList | vscode.CompletionItem[] | undefined
    >('vscode.executeCompletionItemProvider', document.uri, position);
    const items = Array.isArray(result) ? result : result?.items ?? [];
    return items.map(item => typeof item.label === 'string' ? item.label : item.label.label);
}

async function hoverText(document: vscode.TextDocument, position: vscode.Position): Promise<string> {
    const hovers = await vscode.commands.executeCommand<vscode.Hover[]>(
        'vscode.executeHoverProvider',
        document.uri,
        position
    );
    return (hovers ?? []).flatMap(hover => hover.contents).map(content => {
        if (typeof content === 'string') return content;
        if (content instanceof vscode.MarkdownString) return content.value;
        return JSON.stringify(content);
    }).join('\n');
}

async function definitionUris(document: vscode.TextDocument, position: vscode.Position): Promise<string[]> {
    const definitions = await vscode.commands.executeCommand<
        Array<vscode.Location | vscode.LocationLink> | undefined
    >('vscode.executeDefinitionProvider', document.uri, position);
    return (definitions ?? []).map(definition => {
        return 'uri' in definition
            ? definition.uri.toString()
            : definition.targetUri.toString();
    });
}

async function waitFor<T>(
    read: () => Promise<T>,
    predicate: (value: T) => boolean,
    description: string,
    timeoutMs = 30_000
): Promise<T> {
    const deadline = Date.now() + timeoutMs;
    let lastValue: T | undefined;
    while (Date.now() < deadline) {
        lastValue = await read();
        if (predicate(lastValue)) return lastValue;
        await new Promise(resolve => setTimeout(resolve, 100));
    }
    throw new Error(`Timed out waiting for ${description}. Last value: ${JSON.stringify(lastValue)}`);
}

async function selectPackage(packagePath: string, profile: string): Promise<void> {
    const configuration = vscode.workspace.getConfiguration('jenkinsKnowledge');
    await configuration.update('packagePath', packagePath, vscode.ConfigurationTarget.Workspace);
    await configuration.update('profile', profile, vscode.ConfigurationTarget.Workspace);
    await vscode.commands.executeCommand('jenkinsKnowledge.reload');
}

suite('Jenkins Knowledge VS Code extension host acceptance', function () {
    this.timeout(120_000);

    suiteSetup(async () => {
        const configuration = vscode.workspace.getConfiguration('jenkinsKnowledge');
        await configuration.update('serverPath', process.env.JENKINS_LSP_PATH, vscode.ConfigurationTarget.Workspace);
        await configuration.update('packagePath', process.env.JENKINS_KNOWLEDGE_LIVE_PACKAGE, vscode.ConfigurationTarget.Workspace);
        await configuration.update('profile', 'jenkins-2356-deep', vscode.ConfigurationTarget.Workspace);
        await vscode.workspace.getConfiguration('files').update(
            'associations',
            { 'Jenkinsfile.*': 'groovy' },
            vscode.ConfigurationTarget.Workspace
        );

        const extension = vscode.extensions.getExtension('dmji.jenkins-knowledge');
        assert.ok(extension, 'Jenkins Knowledge extension was not loaded by the test host.');
        await extension.activate();
    });

    test('live Jenkins 2.356 package provides Pipeline completion, Shared Library hover and navigation', async () => {
        const document = await writeJenkinsfile(
            'Jenkinsfile.live',
            "node {\n  echo 'from Jenkins 2.356'\n  example1()\n}\n"
        );

        const stepLabels = await waitFor(
            () => completionLabels(document, new vscode.Position(1, 4)),
            labels => labels.includes('echo'),
            'Jenkins 2.356 Pipeline GDSL completion'
        );
        assert.ok(stepLabels.includes('echo'));

        const globalLabels = await waitFor(
            () => completionLabels(document, new vscode.Position(2, 7)),
            labels => labels.includes('example1'),
            'pinned Shared Library global completion'
        );
        assert.ok(globalLabels.includes('example1'));

        const hover = await waitFor(
            () => hoverText(document, new vscode.Position(2, 6)),
            value => value.includes('example1'),
            'Shared Library hover'
        );
        assert.ok(hover.includes('gsa-examples'));

        const definitions = await waitFor(
            () => definitionUris(document, new vscode.Position(2, 6)),
            values => values.some(value => value.includes('7dfe2ab4bf7123b6aea29b0ec92caa2277cea5de')),
            'pinned Shared Library definition'
        );
        assert.ok(definitions.some(value => value.includes('vars/example1.groovy')));
    });

    test('fixture package reports unavailable plugin steps but ignores unknown names, comments and strings', async () => {
        await selectPackage(
            process.env.JENKINS_KNOWLEDGE_PRIMARY_PACKAGE!,
            'contract-primary'
        );
        const document = await writeJenkinsfile(
            'Jenkinsfile.primary',
            "node {\n  unavailableStep()\n  unknownStep()\n  deployApp()\n  // unavailableStep()\n  def text = 'unavailableStep()'\n}\n"
        );

        const diagnostics = await waitFor(
            async () => vscode.languages.getDiagnostics(document.uri),
            values => values.some(diagnostic => diagnostic.code === 'jenkins.plugin.unavailable'),
            'unavailable plugin diagnostic'
        );
        const unavailable = diagnostics.filter(diagnostic => diagnostic.code === 'jenkins.plugin.unavailable');
        assert.strictEqual(unavailable.length, 1);
        assert.strictEqual(unavailable[0].range.start.line, 1);

        const hover = await waitFor(
            () => hoverText(document, new vscode.Position(3, 5)),
            value => value.includes('deployApp'),
            'Shared Library hover in fixture package'
        );
        assert.ok(hover.includes('Fixture Shared Library'));
        assert.ok(hover.includes('verified'));
        const definitions = await definitionUris(document, new vscode.Position(3, 5));
        assert.strictEqual(definitions.length, 0, 'non-GitHub fixture URLs must not become navigation targets');
    });

    test('package reload switches the completion catalog and ordinary Groovy files stay out of Jenkins context', async () => {
        await selectPackage(
            process.env.JENKINS_KNOWLEDGE_ALTERNATE_PACKAGE!,
            'contract-alternate'
        );
        const document = await writeJenkinsfile(
            'Jenkinsfile.alternate',
            'node {\n  alternateOnly()\n}\n'
        );
        const labels = await waitFor(
            () => completionLabels(document, new vscode.Position(1, 8)),
            values => values.includes('alternateOnly'),
            'alternate package completion after reload'
        );
        assert.ok(labels.includes('alternateOnly'));
        assert.ok(!labels.includes('deployApp'));

        const ordinaryUri = vscode.Uri.file(path.join(workspacePath(), 'Ordinary.groovy'));
        await vscode.workspace.fs.writeFile(ordinaryUri, Buffer.from("node {\n  sh 'not a Jenkinsfile'\n}\n", 'utf8'));
        const ordinary = await vscode.workspace.openTextDocument(ordinaryUri);
        assert.strictEqual(ordinary.languageId, 'groovy');
        const ordinaryLabels = await completionLabels(ordinary, new vscode.Position(1, 4));
        assert.ok(!ordinaryLabels.includes('echo'));
        assert.ok(!ordinaryLabels.includes('alternateOnly'));
    });
});
