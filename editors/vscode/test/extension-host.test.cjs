const test = require('node:test');
const assert = require('node:assert/strict');
const Module = require('node:module');

const commands = new Map();
const messages = [];
const configuration = {
    serverPath: 'jenkins-lsp',
    packagePath: './.knowledge',
    profile: 'contract-primary'
};
const vscode = {
    workspace: {
        workspaceFolders: [{ uri: { fsPath: '/workspace' } }],
        getConfiguration: () => ({
            get: (key, fallback) => configuration[key] ?? fallback
        })
    },
    commands: {
        registerCommand: (name, callback) => {
            commands.set(name, callback);
            return { dispose() {} };
        }
    },
    window: {
        showWarningMessage: message => messages.push({ level: 'warning', message }),
        showInformationMessage: message => messages.push({ level: 'info', message }),
        showErrorMessage: message => messages.push({ level: 'error', message })
    }
};

const clients = [];
class FakeLanguageClient {
    constructor(id, name, serverOptions, clientOptions) {
        this.id = id;
        this.name = name;
        this.serverOptions = serverOptions;
        this.clientOptions = clientOptions;
        this.started = false;
        this.stopped = false;
        this.requests = [];
        clients.push(this);
    }

    async start() {
        this.started = true;
    }

    async stop() {
        this.stopped = true;
    }

    async sendRequest(method, params) {
        this.requests.push({ method, params });
        return { status: 'loaded', profile: 'contract-primary' };
    }
}

const originalLoad = Module._load;
Module._load = function (request, parent, isMain) {
    if (request === 'vscode') return vscode;
    if (request === 'vscode-languageclient/node') {
        return { LanguageClient: FakeLanguageClient };
    }
    return originalLoad.call(this, request, parent, isMain);
};

const extension = require('../out/extension.js');
Module._load = originalLoad;

test('activation wires the selected package to LSP and reloads it through the command', async () => {
    const context = { subscriptions: [] };
    await extension.activate(context);

    assert.equal(clients.length, 1);
    const client = clients[0];
    assert.equal(client.id, 'jenkinsKnowledge');
    assert.equal(client.name, 'Jenkins Knowledge');
    assert.equal(client.serverOptions.command, 'jenkins-lsp');
    assert.deepEqual(client.serverOptions.args, []);
    assert.equal(client.serverOptions.options.cwd, '/workspace');
    assert.deepEqual(client.clientOptions.documentSelector, [
        { scheme: 'file', language: 'groovy' }
    ]);
    assert.equal(
        client.clientOptions.initializationOptions.packagePath,
        '/workspace/.knowledge'
    );
    assert.equal(
        client.clientOptions.initializationOptions.profile,
        'contract-primary'
    );
    assert.equal(client.started, true);
    assert.ok(context.subscriptions.includes(client));

    const reload = commands.get('jenkinsKnowledge.reload');
    assert.equal(typeof reload, 'function');
    await reload();

    assert.deepEqual(client.requests, [{
        method: 'jenkinsKnowledge/reload',
        params: {
            packagePath: '/workspace/.knowledge',
            profile: 'contract-primary'
        }
    }]);
    assert.ok(messages.some(item =>
        item.level === 'info' &&
        item.message.includes("loaded profile 'contract-primary'")
    ));

    await extension.deactivate();
    assert.equal(client.stopped, true);
});
