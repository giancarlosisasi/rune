'use strict';

// Test 6b.6 — the publish wrapper retries the signing transparency log and nothing else.
//
// Retry logic is written once, exercised never, and wrong. This is the test that runs it.

const assert = require('node:assert/strict');
const test = require('node:test');

const { BACKOFF_MS, RETRIES, withTransparencyLogRetry } = require('../scripts/publish');

// A failure shaped the way `execFileSync` reports one: the interesting text is on the
// streams, not always in the message.
function npmFailure(output) {
  return Object.assign(new Error('Command failed: npm publish'), {
    stdout: Buffer.from(''),
    stderr: Buffer.from(output),
  });
}

const TLOG = npmFailure('npm error 500 Internal Server Error - TLOG_CREATE_ENTRY_ERROR');

// npm leaves the reason on the streams and the message as "Command failed", so what a
// caller has to see coming back out is the failure itself, not a rephrasing of it.
function reports(pattern) {
  return (error) => pattern.test(String(error.stderr));
}

// A sleep that records what it was asked to wait for instead of waiting for it.
function recorder() {
  const waited = [];
  return { waited, sleep: (ms) => { waited.push(ms); return Promise.resolve(); } };
}

test('6b.6 — a transparency-log failure is retried with backoff, then gives up', async () => {
  const clock = recorder();
  let attempts = 0;

  await assert.rejects(
    withTransparencyLogRetry(
      () => {
        attempts += 1;
        throw TLOG;
      },
      { sleep: clock.sleep },
    ),
    reports(/TLOG_CREATE_ENTRY_ERROR/),
  );

  assert.equal(attempts, RETRIES + 1, 'the first attempt plus every retry');
  assert.deepEqual(clock.waited, BACKOFF_MS.slice(0, RETRIES));
});

test('6b.6 — the wait grows between attempts', () => {
  for (let index = 1; index < BACKOFF_MS.length; index += 1) {
    assert.ok(BACKOFF_MS[index] > BACKOFF_MS[index - 1], 'the backoff does not grow');
  }
});

test('6b.6 — a transparency-log failure that clears is not a release failure', async () => {
  const clock = recorder();
  let attempts = 0;

  const result = await withTransparencyLogRetry(
    () => {
      attempts += 1;
      if (attempts < 3) {
        throw TLOG;
      }
      return 'published';
    },
    { sleep: clock.sleep },
  );

  assert.equal(result, 'published');
  assert.equal(attempts, 3);
  assert.deepEqual(clock.waited, BACKOFF_MS.slice(0, 2));
});

test('6b.6 — any other failure ends the release immediately', async () => {
  const clock = recorder();
  let attempts = 0;

  await assert.rejects(
    withTransparencyLogRetry(
      () => {
        attempts += 1;
        throw npmFailure('npm error 403 You cannot publish over the previously published version');
      },
      { sleep: clock.sleep },
    ),
    reports(/cannot publish over/),
  );

  assert.equal(attempts, 1, 'a version conflict was retried');
  assert.deepEqual(clock.waited, [], 'a version conflict was waited on');
});

test('6b.6 — an authentication failure is not retried into three of itself', async () => {
  let attempts = 0;

  await assert.rejects(
    withTransparencyLogRetry(
      () => {
        attempts += 1;
        throw npmFailure('npm error code ENEEDAUTH');
      },
      { sleep: () => Promise.resolve() },
    ),
    reports(/ENEEDAUTH/),
  );

  assert.equal(attempts, 1);
});

test('6b.6 — the transparency log is recognised by name as well as by code', async () => {
  let attempts = 0;

  await assert.rejects(
    withTransparencyLogRetry(
      () => {
        attempts += 1;
        throw npmFailure('npm error Failed to publish the transparency log entry');
      },
      { sleep: () => Promise.resolve() },
    ),
    reports(/transparency log/),
  );

  assert.equal(attempts, RETRIES + 1);
});

test('6b.6 — a run that never fails never waits', async () => {
  const clock = recorder();

  assert.equal(await withTransparencyLogRetry(() => 'published', { sleep: clock.sleep }), 'published');
  assert.deepEqual(clock.waited, []);
});
