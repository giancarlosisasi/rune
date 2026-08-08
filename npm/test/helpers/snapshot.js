'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const DIRECTORY = path.join(__dirname, '..', 'snapshots');

// A missing snapshot is a failure, never a silent pass. Writing one on first sight
// would make a deleted snapshot file look like a green test.
function assertSnapshot(name, actual) {
  const file = path.join(DIRECTORY, `${name}.txt`);
  const text = typeof actual === 'string' ? actual : `${JSON.stringify(actual, null, 2)}\n`;

  if (process.env.UPDATE_SNAPSHOTS === '1') {
    fs.mkdirSync(DIRECTORY, { recursive: true });
    fs.writeFileSync(file, text);
    return;
  }

  if (!fs.existsSync(file)) {
    assert.fail(`no snapshot at ${file} — run the suite again with UPDATE_SNAPSHOTS=1`);
  }

  assert.equal(text, fs.readFileSync(file, 'utf8'), `snapshot ${name} changed`);
}

module.exports = { assertSnapshot };
