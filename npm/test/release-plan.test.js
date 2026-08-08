'use strict';

// Tests 6b.3 and 6b.4 — the decisions a release makes before it publishes anything,
// exercised without a registry and without a workflow run.

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const platforms = require('../rune/lib/platforms');
const { withDerivedPins } = require('../scripts/bump-version');
const { META, plan, publishOrder, validatePins } = require('../scripts/release-plan');

const committed = JSON.parse(
  fs.readFileSync(path.join(__dirname, '..', 'rune', 'package.json'), 'utf8'),
);

const ALL = platforms.PLATFORMS.map((entry) => entry.package);
const VERSION = '1.4.0';

// The committed manifest as a bump would leave it: the same fixture the release itself
// validates, at the version under test.
const meta = withDerivedPins(committed, VERSION);

// What a build of the whole matrix produces: every platform package, at one version.
function built(version = VERSION) {
  return ALL.map((name) => ({ name, version }));
}

test('6b.3 — a version no registry carries publishes everything', () => {
  const decided = plan({ version: VERSION, published: [] });

  assert.deepEqual(decided.publish, publishOrder());
  assert.deepEqual(decided.skip, []);
});

test('6b.3 — a version already fully published publishes nothing', () => {
  const decided = plan({ version: VERSION, published: publishOrder() });

  assert.deepEqual(decided.publish, []);
  assert.deepEqual(decided.skip, publishOrder());
});

test('6b.3 — a run that stopped halfway is completed, not repeated', () => {
  const done = ALL.slice(0, 3);

  const decided = plan({ version: VERSION, published: done });

  assert.deepEqual(decided.publish, [...ALL.slice(3), META]);
  assert.deepEqual(decided.skip, done);
});

test('6b.3 — a package published at some other version is still to publish', () => {
  // The caller reports what is published *at the target version*. A package sitting at
  // an older version is absent from that list, which is the whole of the answer.
  const decided = plan({ version: VERSION, published: [] });

  assert.ok(decided.publish.includes(ALL[0]));
});

test('6b.4 — every platform package precedes the meta package', () => {
  const ordered = publishOrder();

  assert.equal(ordered.at(-1), META);
  assert.equal(ordered.length, ALL.length + 1);
  for (const name of ALL) {
    assert.ok(ordered.indexOf(name) < ordered.indexOf(META), `${name} is published too late`);
  }
});

test('6b.4 — pins matching the built set exactly are accepted', () => {
  assert.deepEqual(validatePins({ manifest: meta, built: built(), version: VERSION }), []);
});

test('6b.4 — the committed manifest would pass its own release', () => {
  const problems = validatePins({
    manifest: committed,
    built: built(committed.version),
    version: committed.version,
  });

  assert.deepEqual(problems, []);
});

test('6b.4 — a package that was pinned but never built fails the release', () => {
  const missing = built().filter((one) => one.name !== ALL[2]);

  const problems = validatePins({ manifest: meta, built: missing, version: VERSION });

  assert.equal(problems.length, 1);
  assert.match(problems[0], new RegExp(`${ALL[2]}.*not built`));
});

test('6b.4 — a package that was built but never pinned fails the release', () => {
  const extra = [...built(), { name: '@giancarlosio/rune-sunos-x64', version: VERSION }];

  const problems = validatePins({ manifest: meta, built: extra, version: VERSION });

  assert.equal(problems.length, 1);
  assert.match(problems[0], /rune-sunos-x64.*not pinned/);
});

test('6b.4 — a built package at the wrong version fails the release', () => {
  const drifted = built().map((one) => (one.name === ALL[0] ? { ...one, version: '1.3.9' } : one));

  const problems = validatePins({ manifest: meta, built: drifted, version: VERSION });

  assert.equal(problems.length, 1);
  assert.match(problems[0], /1\.3\.9/);
});

test('6b.4 — a pin left behind at the previous version fails the release', () => {
  const stale = {
    ...meta,
    optionalDependencies: { ...meta.optionalDependencies, [ALL[1]]: '0.0.1' },
  };

  const problems = validatePins({ manifest: stale, built: built(), version: VERSION });

  // The pin is wrong, and so is the manifest version the pins were derived from.
  assert.ok(problems.some((problem) => problem.includes('0.0.1')));
});

test('6b.4 — the meta package must carry the version being released', () => {
  const problems = validatePins({ manifest: meta, built: built(), version: '9.9.9' });

  assert.ok(problems.some((problem) => problem.includes('9.9.9')));
});
