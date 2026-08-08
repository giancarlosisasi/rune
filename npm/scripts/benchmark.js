'use strict';

// Gate G8 — the performance claim, measured rather than asserted in a document.
//
// Rune's stated value is that it adds single-digit milliseconds to the command it runs.
// An unenforced claim decays: nobody notices the run that took 40 ms instead of 4 ms
// until a user does. This blocks the release.
//
// What is measured is a **warm** `rune list`: the config cache is hit and no child is
// spawned, so the number is rune's own overhead and nothing else's. Measuring a
// `rune run` instead would measure the shell and the child, which are the user's cost
// either way and would drown the thing under test.

const assert = require('node:assert/strict');
const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

// The documented budget: binary startup under 3 ms plus a cache hit under 2 ms.
const BUDGET_MS = 5;

const FIXTURE = path.join(__dirname, '..', 'test', 'fixtures', 'monorepo');

function project() {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'rune-bench-'));
  fs.cpSync(FIXTURE, directory, { recursive: true });

  // The cache lives under node_modules, and an install is not otherwise needed here.
  fs.mkdirSync(path.join(directory, 'node_modules'), { recursive: true });
  return directory;
}

function measure(binary, cwd) {
  const report = path.join(cwd, 'hyperfine.json');

  // `-N` skips the intermediate shell. Without it the shell's own startup is most of
  // what gets timed, and the result says nothing about rune.
  const run = spawnSync(
    'hyperfine',
    ['-N', '--warmup', '20', '--runs', '200', '--export-json', report, `${binary} list`],
    { cwd, encoding: 'utf8', stdio: ['ignore', 'inherit', 'inherit'] },
  );
  assert.equal(run.status, 0, 'hyperfine did not finish');

  const [result] = JSON.parse(fs.readFileSync(report, 'utf8')).results;
  return { mean: result.mean * 1000, stddev: result.stddev * 1000 };
}

function benchmark(binary) {
  const cwd = project();

  try {
    // One run outside the measurement, so what is timed is the cache hit and never the
    // one cold evaluation that fills it.
    const warm = spawnSync(binary, ['list'], { cwd, encoding: 'utf8' });
    assert.equal(warm.status, 0, `rune list failed before the benchmark:\n${warm.stderr}`);

    const { mean, stddev } = measure(binary, cwd);
    const verdict = mean <= BUDGET_MS ? 'within' : 'over';
    const line = `warm \`rune list\`: ${mean.toFixed(2)} ms ± ${stddev.toFixed(2)} ms — ${verdict} the ${BUDGET_MS} ms budget`;

    process.stdout.write(`${line}\n`);
    if (process.env.GITHUB_STEP_SUMMARY) {
      fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, `${line}\n`);
    }

    return mean <= BUDGET_MS;
  } finally {
    fs.rmSync(cwd, { recursive: true, force: true });
  }
}

if (require.main === module) {
  const [binary = path.join(__dirname, '..', '..', 'target', 'release', 'rune')] =
    process.argv.slice(2);

  if (!benchmark(binary)) {
    process.exitCode = 1;
  }
}

module.exports = { BUDGET_MS, benchmark };
