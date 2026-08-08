'use strict';

// The part of the README that is generated stays generated.
//
// A platform added to the table has to reach the page a user reads, and the only way to
// guarantee that is to fail when it has not.

const assert = require('node:assert/strict');
const fs = require('node:fs');
const test = require('node:test');

const { README, render, table } = require('../scripts/readme-platforms');

test('the README platform matrix is what the platform table renders', () => {
  const readme = fs.readFileSync(README, 'utf8');

  assert.equal(
    readme,
    render(readme),
    'run `node npm/scripts/readme-platforms.js` to bring the README up to date',
  );
});

test('every published platform package has a row', () => {
  const rendered = table();

  for (const entry of require('../rune/lib/platforms').PLATFORMS) {
    assert.ok(rendered.includes(entry.package), `${entry.package} is missing from the README`);
  }
});

// The one thing about rune that a user cannot deduce and that fails silently.
test('the README says the config file must go in a task runner\'s inputs', () => {
  const readme = fs.readFileSync(README, 'utf8');

  assert.match(readme, /rune\.config\.ts.*inputs/is);
  assert.match(readme, /turbo/i);
});
