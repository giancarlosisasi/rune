'use strict';

// Creating the registry entry for a package that does not exist yet.
//
// npm will only let a trusted publisher be configured for a package that is already
// there, and a trusted publisher is the only way this repository can publish. So the
// name has to exist before the pipeline can ever use it, and the only thing that can
// create it is a person with a token.
//
// What gets published is a placeholder at 0.0.0 under the `bootstrap` dist-tag: a
// manifest and a README, no binary, no `bin`, nothing that resolves.
//
// `--tag bootstrap` does not keep `latest` empty. npm points `latest` at the first version
// of a new package whatever tag is asked for, so 0.0.0 holds `latest` until the first real
// release moves it. What that buys is time, not safety: install the placeholder and you get
// a manifest that does nothing, rather than a broken binary.
//
// This is also what runs when a seventh platform package is added.

const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const platforms = require('../rune/lib/platforms');
const { runNpm } = require('./npm-cli');
const { META, publishOrder } = require('./release-plan');

const PLACEHOLDER = '0.0.0';
const REPOSITORY = 'https://github.com/giancarlosisasi/rune';

function exists(name) {
  try {
    runNpm(['view', name, 'name']);
    return true;
  } catch (error) {
    const said = `${error.stdout ?? ''}${error.stderr ?? ''}`;
    if (/E404/.test(said)) {
      return false;
    }
    throw error;
  }
}

function placeholder(directory, name) {
  const target = path.join(directory, name.replace('/', '-').replace('@', ''));
  fs.mkdirSync(target, { recursive: true });

  const real = name === META ? name : `${META}, which depends on it`;
  fs.writeFileSync(
    path.join(target, 'package.json'),
    `${JSON.stringify(
      {
        name,
        version: PLACEHOLDER,
        description: `Reserves the name. The first real release of ${name} is 0.1.0.`,
        homepage: REPOSITORY,
        repository: { type: 'git', url: `git+${REPOSITORY}.git` },
        license: 'MIT',
        publishConfig: { access: 'public' },
      },
      null,
      2,
    )}\n`,
  );
  fs.writeFileSync(
    path.join(target, 'README.md'),
    `# ${name}\n\nThis version is a placeholder. It exists so that trusted publishing could be\nconfigured for this name before anything real was published. Install ${real}.\n`,
  );

  return target;
}

function main() {
  const dryRun = process.argv.includes('--dry-run');
  const work = fs.mkdtempSync(path.join(os.tmpdir(), 'rune-bootstrap-'));

  try {
    for (const name of publishOrder()) {
      if (exists(name)) {
        process.stdout.write(`${name} is already on the registry\n`);
        continue;
      }

      if (dryRun) {
        process.stdout.write(`would create ${name}@${PLACEHOLDER}\n`);
        continue;
      }

      // Publishing by folder, not by moving into it. npm reads the registry credentials
      // from where it runs, and a temporary directory holds no `.npmrc` of any kind — a
      // token placed in the repository would be ignored and the publish would fail as
      // anonymous.
      runNpm(['publish', placeholder(work, name), '--access', 'public', '--tag', 'bootstrap']);
      process.stdout.write(`created ${name}@${PLACEHOLDER}\n`);
    }
  } finally {
    fs.rmSync(work, { recursive: true, force: true });
  }

  process.stdout.write(
    `\n${platforms.PLATFORMS.length + 1} names checked. Configure a trusted publisher for each one,\n` +
      'then revoke the token that ran this.\n',
  );
}

if (require.main === module) {
  main();
}

module.exports = { PLACEHOLDER, exists, placeholder };
