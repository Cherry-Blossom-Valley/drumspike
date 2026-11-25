// Loads the script-tag build of the LUNA Builder SDK (vendor/luna.global.js, fetched by tools/fetch-sdk.mjs) into Node,
// so the tests and the command line tools need no npm install. The build is a classic script that
// defines `var Luna`; it runs in this context and the result is read from the global.
import { readFileSync } from 'node:fs';
import { runInThisContext } from 'node:vm';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));

export function loadLuna() {
  if (!globalThis.Luna) {
    let source;
    try {
      source = readFileSync(`${root}vendor/luna.global.js`, 'utf8');
    } catch {
      throw new Error('vendor/luna.global.js is missing: the SDK build is not in this repository. Run: npm run fetch-sdk -- <path to luna-builder-sdk> (see vendor/README.md)');
    }
    globalThis.window ??= globalThis;
    runInThisContext(`${source}\nglobalThis.Luna = Luna;`, { filename: 'luna.global.js' });
  }
  return globalThis.Luna;
}

/** Loads logic.js (a classic script for the page) and returns its `DrumSpikeDeliver` global. */
export function loadLogic() {
  loadLuna();
  if (!globalThis.DrumSpikeDeliver) {
    runInThisContext(readFileSync(`${root}logic.js`, 'utf8'), { filename: 'logic.js' });
  }
  return globalThis.DrumSpikeDeliver;
}
