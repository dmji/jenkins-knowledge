const vscode = require('vscode');
const assert = require('node:assert/strict');
const path = require('node:path');
const { setTimeout: delay } = require('node:timers/promises');

async function waitFor(description, predicate, timeoutMs = 45000) {
    const deadline = Date.now() + timeoutMs;
    let lastError;
    while (Date.now() < deadline) {
        try {
            const value = await predicate();
            if (value) return value;
        } catch (error) {
            lastError = error;
        }
        await delay(250);
    }
    throw new Error(`Timed out waiting for ${description}${lastError ? `: ${lastError}` : ''}`);
}

async function completions(document, line, character) {
    return vscode.commands.executeCommand(
        'vscode.executeCompletionItemProvider',
        document.uri,
        new vscode.Position(line, character)
    );
}

function hoverText(items) {
    return (items || []).flatMap(item => item.contents || [])
        .map(content => typeof content === 'string' ? content : content.value || content.toString())
        .join('\n');
}

async function activateExtension() {
    const extension = vscode.extensions.getExtension('dmji.jenkins-knowledge');
    assert.ok(extension, 'Jenkins Knowledge extension must be loaded in the extension host');
    await extension.activate();
    assert.equal(extension.isActive, true);
}

async function run() {
    await activateExtension();

    const livePackage = vscode.workspace.getConfiguration('jenkinsKnowledge').get('packagePath');
    const fixturePackage = process.env.JENKINS_KNOWLEDGE_FIXTURE_PACKAGE;
    const alternatePackage = process.env.JENKINS_KNOWLEDGE_ALTERNATE_PACKAGE;
    assert.ok(livePackage && path.isAbsolute(livePackage), 'live package path must be absolute');
    assert.ok(fixturePackage && path.isAbsolute(fixturePackage), 'fixture package path must be absolute');
    assert.ok(alternatePackage && path.isAbsolute(alternatePackage), 'alternate package path must be absolute');

    const jenkinsfile = path.join(vscode.workspace.workspaceFolders[0].uri.fsPath, 'Jenkinsfile');
    const document = await vscode.workspace.openTextDocument(vscode.Uri.file(jenkinsfile));
    await vscode.window.showTextDocument(document);

    const liveCompletion = await waitFor('Pipeline completion from the live Jenkins package', async () => {
        const result = await completions(document, 1, 4);
        return result && result.items.some(item => item.label === 'sh') ? result : undefined;
    });
    assert.ok(liveCompletion.items.some(item => item.label === 'sh'));

    const liveHover = await waitFor('hover from the live Jenkins package', async () => {
        const result = await vscode.commands.executeCommand(
            'vscode.executeHoverProvider',
            document.uri,
            new vscode.Position(1, 3)
        );
        const text = hoverText(result);
        return text.trim() ? text : undefined;
    });
    assert.match(liveHover, /sh/);

    const configuration = vscode.workspace.getConfiguration('jenkinsKnowledge');
    await configuration.update('packagePath', fixturePackage, vscode.ConfigurationTarget.Workspace);
    await configuration.update('profile', 'contract-primary', vscode.ConfigurationTarget.Workspace);
    await vscode.commands.executeCommand('jenkinsKnowledge.reload');

    await waitFor('fixture package completion after explicit reload', async () => {
        const result = await completions(document, 3, 4);
        return result && result.items.some(item => item.label === 'deployApp') ? result : undefined;
    });

    const deployHover = await waitFor('Shared Library hover', async () => {
        const result = await vscode.commands.executeCommand(
            'vscode.executeHoverProvider',
            document.uri,
            new vscode.Position(3, 6)
        );
        const text = hoverText(result);
        return text.includes('deployApp') ? text : undefined;
    });
    assert.match(deployHover, /verified/i);
    assert.match(deployHover, /Fixture Shared Library/);

    const definitions = await vscode.commands.executeCommand(
        'vscode.executeDefinitionProvider',
        document.uri,
        new vscode.Position(3, 6)
    );
    assert.ok(Array.isArray(definitions) && definitions.length > 0, 'Shared Library symbol must have a definition');
    assert.match(definitions[0].uri.toString(), /0123456789012345678901234567890123456789\/vars\/deployApp\.groovy#L1$/);

    const pluginDiagnostics = await waitFor('unavailable plugin diagnostic', () => {
        const diagnostics = vscode.languages.getDiagnostics(document.uri);
        return diagnostics.some(diagnostic => diagnostic.code === 'jenkins.plugin.unavailable')
            ? diagnostics
            : undefined;
    });
    assert.ok(pluginDiagnostics.some(diagnostic =>
        diagnostic.code === 'jenkins.plugin.unavailable' && diagnostic.range.start.line === 2
    ));
    assert.ok(!pluginDiagnostics.some(diagnostic =>
        [5, 6, 7].includes(diagnostic.range.start.line)
    ), 'comments, strings, and unknown names must not produce Jenkins diagnostics');

    await configuration.update('packagePath', alternatePackage, vscode.ConfigurationTarget.Workspace);
    await configuration.update('profile', 'contract-alternate', vscode.ConfigurationTarget.Workspace);
    await vscode.commands.executeCommand('jenkinsKnowledge.reload');
    const alternateCompletion = await waitFor('alternate package completion after reload', async () => {
        const result = await completions(document, 4, 4);
        return result && result.items.some(item => item.label === 'alternateOnly') ? result : undefined;
    });
    assert.ok(alternateCompletion.items.some(item => item.label === 'alternateOnly'));
    const isolatedCompletion = await completions(document, 3, 4);
    assert.ok(!isolatedCompletion.items.some(item => item.label === 'deployApp'),
        'symbols from the previous package must not leak after reload');

    const ordinaryPath = path.join(vscode.workspace.workspaceFolders[0].uri.fsPath, 'ordinary.groovy');
    const ordinary = await vscode.workspace.openTextDocument(vscode.Uri.file(ordinaryPath));
    await vscode.window.showTextDocument(ordinary);
    const ordinaryCompletions = await completions(ordinary, 0, 8);
    const ordinaryItems = ordinaryCompletions?.items || ordinaryCompletions || [];
    assert.ok(!ordinaryItems.some(item => item.label === 'alternateOnly'),
        'ordinary Groovy files must not receive Jenkins catalog completions');
    const ordinaryHover = await vscode.commands.executeCommand(
        'vscode.executeHoverProvider',
        ordinary.uri,
        new vscode.Position(0, 8)
    );
    assert.ok(!ordinaryHover || ordinaryHover.length === 0,
        'ordinary Groovy files must not receive Jenkins catalog hover');

    console.log('VS Code extension-host acceptance passed: live Jenkins package, fixture reload, completion, hover, definition, diagnostics, and ordinary Groovy isolation.');
}

module.exports = { run };
