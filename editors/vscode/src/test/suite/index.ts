import * as path from 'path';
import Mocha from 'mocha';

export function run(): Promise<void> {
    const mocha = new Mocha({
        ui: 'tdd',
        timeout: 120_000
    });
    const testsRoot = path.resolve(__dirname);
    mocha.addFile(path.resolve(testsRoot, 'extension.test.js'));

    return new Promise((resolve, reject) => {
        mocha.run(failures => {
            if (failures > 0) {
                reject(new Error(`${failures} VS Code extension-host test(s) failed.`));
            } else {
                resolve();
            }
        });
    });
}
