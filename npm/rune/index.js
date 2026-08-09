'use strict';

// What a config imports, for everything that is not rune itself.
//
// rune evaluates a config with an embedded engine that has no npm resolution, so these
// imports are satisfied inside the binary rather than from node_modules. This file is
// what any other tool that reads the config — an editor, a bundler, a test that imports
// it — gets when it follows the same import for real.

// `defineConfig` is types and nothing else: it hands back what it was given.
function defineConfig(config) {
  return config;
}

// Anything other than empty, `0` or `false` counts as CI. The binary decides this the
// same way, so a config branches identically wherever it is read from.
function isCI() {
  const value = process.env.CI;
  return value !== undefined && value !== '' && value !== '0' && value !== 'false';
}

// `isCI` is an accessor here for the same reason it is one inside the binary: it derives
// from an environment variable, so reading it has to see the environment as it is now
// rather than as it was when this module first loaded.
const rune = Object.freeze({
  env: process.env,
  platform: process.platform,
  get isCI() {
    return isCI();
  },
});

module.exports = { defineConfig, rune };
