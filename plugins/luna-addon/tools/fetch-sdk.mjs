// Copies the script-tag build of the LUNA Builder SDK from a local copy of the vendor's repository into vendor/.
// The build is not kept in this repository (see vendor/README.md). Usage: node tools/fetch-sdk.mjs <path to luna-builder-sdk>
import { copyFileSync, existsSync, statSync } from 'node:fs';
import { resolve } from 'node:path';

const root = new URL('..', import.meta.url).pathname;
const from = process.argv[2] || process.env.LUNA_BUILDER_SDK;
if (!from) {
  console.error('usage: npm run fetch-sdk -- <path to a local copy of uaudio-labs/luna-builder-sdk> (or set LUNA_BUILDER_SDK)');
  process.exit(2);
}
const candidates = ['plugins/ua-luna-builder/sdk/luna.global.js', 'dist/luna.global.js', 'luna.global.js'].map((p) => resolve(from, p));
const source = candidates.find((p) => existsSync(p) && statSync(p).isFile());
if (!source) {
  console.error(`no luna.global.js under ${from} (looked for: ${candidates.join(', ')})`);
  process.exit(1);
}
const target = `${root}vendor/luna.global.js`;
copyFileSync(source, target);
console.log(`copied the SDK build (${statSync(target).size} bytes) to vendor/luna.global.js`);
