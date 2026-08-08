'use strict';

// A release, rehearsed. Every decision the pipeline makes runs here, against this
// machine's real artifacts and the real registry, and nothing is published.
//
// This is what the four pure functions were extracted for. A release whose logic exists
// only inside a workflow can be rehearsed only by releasing.

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const platforms = require('../rune/lib/platforms');
const { PACKED, version } = require('./dist');
const { META, plan, publishOrder, validatePins } = require('./release-plan');
const { isPublished, withTransparencyLogRetry } = require('./publish');

const TARBALLS = path.join(__dirname, '..', 'dist', 'tarballs');

function say(step, detail) {
  process.stdout.write(`${step.padEnd(18)} ${detail}\n`);
}

// The pins, against the set the build matrix will produce. Checking them against what
// this machine happened to pack would only ever prove that one package is consistent.
function pins(release) {
  const manifest = JSON.parse(
    fs.readFileSync(path.join(__dirname, '..', 'rune', 'package.json'), 'utf8'),
  );
  const built = platforms.PLATFORMS.map((entry) => ({ name: entry.package, version: release }));

  const problems = validatePins({ manifest, built, version: release });
  assert.deepEqual(problems, [], `the pins do not describe the release:\n  ${problems.join('\n  ')}`);

  say('pins', `${built.length} packages pinned at ${release}, and the meta package agrees`);
}

// What the gate would decide right now. Read-only: `npm view` and nothing else.
function gate(release) {
  const published = publishOrder().filter((name) => isPublished(name, release));
  const decided = plan({ version: release, published });

  say('gate', `${decided.publish.length} to publish, ${decided.skip.length} already there`);
  for (const name of decided.publish) {
    say('', `  would publish ${name}@${release}`);
  }
}

// What `just dist` left behind, checked the way the pipeline checks it.
function packaging(release) {
  const record = path.join(TARBALLS, PACKED);
  assert.ok(fs.existsSync(record), `no ${PACKED} in ${TARBALLS} — run \`just dist\` first`);

  const packed = JSON.parse(fs.readFileSync(record, 'utf8'));
  assert.equal(packed.version, release, 'the packed tarballs are of another version');

  for (const one of packed.packed) {
    assert.ok(fs.existsSync(path.join(TARBALLS, one.tarball)), `${one.name} has no tarball`);
  }

  const platform = packed.packed.find((one) => one.name !== META);
  const entry = platforms.PLATFORMS.find((one) => one.package === platform.name);
  const binary = path.join(
    __dirname,
    '..',
    'dist',
    path.basename(entry.package),
    platforms.BINARY_DIRECTORY,
    entry.binary,
  );

  const executable = process.platform === 'win32' || (fs.statSync(binary).mode & 0o111) !== 0;
  assert.ok(executable, `${binary} is not executable`);

  say('packaging', `${packed.packed.length} tarballs, binary executable`);
}

// The retry wrapper, driven through both of its branches with no waiting and no npm.
async function retry() {
  const tlog = Object.assign(new Error('Command failed'), {
    stderr: 'npm error TLOG_CREATE_ENTRY_ERROR',
  });
  const sleep = () => Promise.resolve();

  let attempts = 0;
  await assert.rejects(
    withTransparencyLogRetry(() => {
      attempts += 1;
      throw tlog;
    }, { sleep }),
  );
  assert.ok(attempts > 1, 'a transparency-log failure was not retried');

  let once = 0;
  await assert.rejects(
    withTransparencyLogRetry(() => {
      once += 1;
      throw Object.assign(new Error('Command failed'), { stderr: 'npm error ENEEDAUTH' });
    }, { sleep }),
  );
  assert.equal(once, 1, 'an authentication failure was retried');

  say('retry', `${attempts} attempts on the transparency log, ${once} on anything else`);
}

async function main() {
  const release = version();
  say('version', release);

  pins(release);
  packaging(release);
  gate(release);
  await retry();

  process.stdout.write('\nnothing was published.\n');
}

if (require.main === module) {
  main().catch((error) => {
    process.exitCode = 1;
    process.stderr.write(`${error.message}\n`);
  });
}
