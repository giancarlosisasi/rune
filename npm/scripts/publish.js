'use strict';

// Publishing, and the one failure that is worth retrying.
//
// npm signs every tarball through Sigstore, and the transparency log that records the
// signature is intermittently unavailable. That is the only failure a retry can fix. An
// authentication failure retried is three authentication failures, and a version
// conflict retried is three of those — one clear error turned into three confusing ones.

const fs = require('node:fs');
const path = require('node:path');

const { runNpm } = require('./npm-cli');
const { PACKED } = require('./dist');
const { plan, validatePins } = require('./release-plan');

// npm reports it by code; a registry proxy in front of it may only say it in words.
const TRANSPARENCY_LOG = /TLOG_CREATE_ENTRY_ERROR|transparency log/i;

// How many times a transparency-log failure is tried again after the first attempt.
const RETRIES = 3;

// One wait per retry, growing, so a log that is briefly overloaded is given longer each
// time rather than being asked three times in the same second.
const BACKOFF_MS = [1000, 2000, 4000];

// Provenance publishing needs at least this, and trusted publishing needs provenance.
const MINIMUM_NPM = '11.5.1';

// Everything npm said, wherever it said it. `execFileSync` puts the interesting text on
// the streams and leaves the message as "Command failed".
function transcript(error) {
  return ['message', 'stdout', 'stderr']
    .map((field) => error?.[field])
    .filter((part) => part !== undefined && part !== null)
    .join('\n');
}

function isTransparencyLogFailure(error) {
  return TRANSPARENCY_LOG.test(transcript(error));
}

async function withTransparencyLogRetry(attempt, { retries = RETRIES, sleep = wait } = {}) {
  for (let tries = 0; ; tries += 1) {
    try {
      return await attempt();
    } catch (error) {
      if (tries >= retries || !isTransparencyLogFailure(error)) {
        throw error;
      }
      await sleep(BACKOFF_MS[Math.min(tries, BACKOFF_MS.length - 1)]);
    }
  }
}

function wait(ms) {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}

// Whether the registry already carries this exact version.
//
// A missing package and a missing version are both "not published". Anything else —
// a network failure, a registry outage — must not be read that way: it would send the
// release into publishing over something that is already there.
function isPublished(name, version) {
  try {
    return String(runNpm(['view', `${name}@${version}`, 'version'])).trim() !== '';
  } catch (error) {
    if (/E404/.test(transcript(error))) {
      return false;
    }
    throw error;
  }
}

function atLeast(actual, minimum) {
  const [left, right] = [actual, minimum].map((text) => text.split('.').map(Number));

  for (let index = 0; index < right.length; index += 1) {
    if ((left[index] ?? 0) !== right[index]) {
      return (left[index] ?? 0) > right[index];
    }
  }
  return true;
}

function assertNpmVersion() {
  const actual = String(runNpm(['--version'])).trim();

  if (!atLeast(actual, MINIMUM_NPM)) {
    throw new Error(`npm ${actual} cannot attach provenance; ${MINIMUM_NPM} or later is required`);
  }
  return actual;
}

// What the assemble step packed, which is also the built set the pins are checked against.
function readPacked(directory) {
  return JSON.parse(fs.readFileSync(path.join(directory, PACKED), 'utf8'));
}

async function publish({ directory, dryRun = false }) {
  const { version, packed } = readPacked(directory);
  const manifest = JSON.parse(
    fs.readFileSync(path.join(__dirname, '..', 'rune', 'package.json'), 'utf8'),
  );

  const built = packed.filter((one) => one.name !== manifest.name);
  const problems = validatePins({ manifest, built, version });
  if (problems.length > 0) {
    throw new Error(`the release does not describe itself:\n  ${problems.join('\n  ')}`);
  }

  const npm = assertNpmVersion();
  const published = packed.filter((one) => isPublished(one.name, version)).map((one) => one.name);
  const decided = plan({ version, published });

  for (const name of decided.skip) {
    process.stdout.write(`${name}@${version} is already published\n`);
  }

  const tarballs = new Map(packed.map((one) => [one.name, one.tarball]));
  for (const name of decided.publish) {
    const tarball = path.join(directory, tarballs.get(name));
    process.stdout.write(`publishing ${name}@${version} with npm ${npm}\n`);

    if (dryRun) {
      continue;
    }
    // A packed tarball rather than a directory: the pack-then-publish split is what
    // makes npm locate the right configuration file.
    await withTransparencyLogRetry(() =>
      runNpm(['publish', tarball, '--provenance', '--access', 'public']),
    );
  }

  return decided;
}

async function main() {
  const args = process.argv.slice(2);
  const [directory = path.join(__dirname, '..', 'dist', 'tarballs')] = args.filter(
    (argument) => !argument.startsWith('--'),
  );

  const decided = await publish({ directory, dryRun: args.includes('--dry-run') });

  process.stdout.write(`published ${decided.publish.length} of ${decided.publish.length + decided.skip.length}\n`);
}

if (require.main === module) {
  main().catch((error) => {
    process.exitCode = 1;
    process.stderr.write(`${transcript(error)}\n`);
  });
}

module.exports = {
  BACKOFF_MS,
  MINIMUM_NPM,
  RETRIES,
  isPublished,
  isTransparencyLogFailure,
  publish,
  withTransparencyLogRetry,
};
