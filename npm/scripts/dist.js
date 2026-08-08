'use strict';

// Assembling the packages that get published, and packing them into tarballs.
//
// One implementation serves both the local `just dist` and the release pipeline, because
// a local dry run that assembles things differently predicts nothing about the pipeline.
// The executable bit is set here rather than inherited: mode bits do not survive an
// upload and download of a build artifact.

const fs = require('node:fs');
const path = require('node:path');

const platforms = require('../rune/lib/platforms');
const { runNpm } = require('./npm-cli');

const META_SOURCE = path.join(__dirname, '..', 'rune');
const WORKSPACE = path.join(__dirname, '..', '..');
const DEFAULT_OUTPUT = path.join(__dirname, '..', 'dist');

const EXECUTABLE = 0o755;

function version() {
  return fs.readFileSync(path.join(WORKSPACE, 'version.txt'), 'utf8').split('\n')[0].trim();
}

// The meta package is copied as it is committed: what is here is what is published.
function assembleMeta(outDirectory) {
  const target = path.join(outDirectory, 'rune');
  fs.rmSync(target, { recursive: true, force: true });
  fs.mkdirSync(target, { recursive: true });

  const manifest = JSON.parse(fs.readFileSync(path.join(META_SOURCE, 'package.json'), 'utf8'));
  for (const entry of manifest.files) {
    fs.cpSync(path.join(META_SOURCE, entry), path.join(target, entry), { recursive: true });
  }
  fs.cpSync(path.join(META_SOURCE, 'package.json'), path.join(target, 'package.json'));
  fs.cpSync(path.join(META_SOURCE, 'README.md'), path.join(target, 'README.md'));

  return target;
}

// A platform package is nothing but its generated manifest and the executable.
function assemblePlatform(outDirectory, entry, binaryPath, packageVersion) {
  const target = path.join(outDirectory, path.basename(entry.package));
  fs.rmSync(target, { recursive: true, force: true });
  fs.mkdirSync(path.join(target, platforms.BINARY_DIRECTORY), { recursive: true });

  fs.writeFileSync(
    path.join(target, 'package.json'),
    `${JSON.stringify(platforms.manifest(entry, packageVersion), null, 2)}\n`,
  );

  const binary = path.join(target, platforms.BINARY_DIRECTORY, entry.binary);
  fs.copyFileSync(binaryPath, binary);
  fs.chmodSync(binary, EXECUTABLE);

  return target;
}

function pack(directory, destination) {
  fs.mkdirSync(destination, { recursive: true });
  runNpm(['pack', '--pack-destination', destination], { cwd: directory });
}

// What `just dist` runs: the host's own release build, packed the way the pipeline packs
// the five it builds.
function main() {
  const entry = platforms.entryFor(process.platform, process.arch);
  if (!entry) {
    throw new Error(`no platform package for ${process.platform} ${process.arch}`);
  }

  const built = path.join(WORKSPACE, 'target', 'release', entry.binary);
  if (!fs.existsSync(built)) {
    throw new Error(`no release binary at ${built} — run \`cargo build --release\` first`);
  }

  const outDirectory = DEFAULT_OUTPUT;
  const tarballs = path.join(outDirectory, 'tarballs');
  fs.rmSync(tarballs, { recursive: true, force: true });

  const packaged = [
    assemblePlatform(outDirectory, entry, built, version()),
    assembleMeta(outDirectory),
  ];
  for (const directory of packaged) {
    pack(directory, tarballs);
  }

  process.stdout.write(`packed ${fs.readdirSync(tarballs).join(', ')}\ninto ${tarballs}\n`);
}

if (require.main === module) {
  main();
}

module.exports = { assembleMeta, assemblePlatform, pack, version };
