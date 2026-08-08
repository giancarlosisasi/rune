'use strict';

// `defineConfig` is types and nothing else: it hands back what it was given.
//
// rune evaluates a config with an embedded engine that has no npm resolution, so the
// import of this module is satisfied inside the binary rather than from node_modules.
// This file is what any other tool that reads the config — an editor, a bundler, a test
// that imports it — gets when it follows the same import for real.

function defineConfig(config) {
  return config;
}

module.exports = { defineConfig };
