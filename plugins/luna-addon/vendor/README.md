# The SDK build is not in this repository

The Add On needs one file of the LUNA Builder SDK of Universal Audio, Inc.: `luna.global.js` (the script-tag build, version 3.1.2, tested with commit 62b8d38 of `uaudio-labs/luna-builder-sdk`).

The file is not kept here. It is under the Apache License 2.0 plus the vendor's "Supplemental Terms for LUNA Builder SDK"; read both in the vendor's repository before you use it.

Get it once per checkout, from a local copy of the vendor's repository:

```sh
npm run fetch-sdk -- /path/to/luna-builder-sdk
```

This copies `plugins/ua-luna-builder/sdk/luna.global.js` into this folder. Git ignores it. The tests and the page fail with a clear text while the file is missing.
