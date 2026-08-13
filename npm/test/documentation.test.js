'use strict';

// Test R25.7 — what a config author is told about waiting, and about what an evaluation
// may spend.
//
// Half of asynchrony working is worse than none of it working when nothing says which
// half. Someone who reaches for `await` gets a working result, reaches one step further
// for a timer, and meets a refusal in a vocabulary the product never taught them.

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const EVALUATION = path.join(__dirname, '..', '..', 'website', 'docs', 'config', 'evaluation.mdx');

const page = () => fs.readFileSync(EVALUATION, 'utf8');

test('the evaluation page states that await and Promise work', () => {
  const documented = page();

  assert.match(documented, /`await` and `Promise` work/);
  assert.match(documented, /top level/i);
});

test('the evaluation page states the rule that makes waiting finite', () => {
  const documented = page();

  assert.match(documented, /evaluated to completion before any script starts/);
  assert.match(documented, /nothing defers/i);
});

test('the evaluation page names both ceilings and the variable that raises each', () => {
  const documented = page();

  for (const stated of ['RUNE_CONFIG_TIME_LIMIT_MS', 'RUNE_CONFIG_MEMORY_LIMIT_MB']) {
    assert.ok(documented.includes(stated), `${stated} is not on the evaluation page`);
  }
});
