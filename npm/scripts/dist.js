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

// What was packed, written beside the tarballs. The publish step reads it rather than
// working the file names out again from the version and the package names.
const PACKED = 'packed.json';

// The repository's single licence, staged into every package the way the readme already
// is. Seven copies committed would be seven files to keep in step, and the one that drifts
// is the one nobody reads. A manifest claiming a licence whose text is nowhere fails some
// scanners outright and has to be explained to every security team that looks.
const LICENSE_FILE = 'LICENSE';

const EXECUTABLE = 0o755;

function version() {
  return fs.readFileSync(path.join(WORKSPACE, 'version.txt'), 'utf8').split('\n')[0].trim();
}

// The meta package is copied as it is committed: what is here is what is published, minus
// the one field that provably cannot work where it lands.
function assembleMeta(outDirectory) {
  const target = path.join(outDirectory, 'rune');
  fs.rmSync(target, { recursive: true, force: true });
  fs.mkdirSync(target, { recursive: true });

  const manifest = JSON.parse(fs.readFileSync(path.join(META_SOURCE, 'package.json'), 'utf8'));
  for (const entry of manifest.files) {
    fs.cpSync(path.join(META_SOURCE, entry), path.join(target, entry), { recursive: true });
  }

  // The committed manifest keeps its scripts so a maintainer bumping locally still gets
  // the derived pins. They name a path above the package root, which is not in the
  // tarball, so published they could only ever fail — and a lifecycle script that cannot
  // run is something a security team has to rule out before anyone installs anything.
  delete manifest.scripts;
  fs.writeFileSync(
    path.join(target, 'package.json'),
    `${JSON.stringify(manifest, null, 2)}\n`,
  );
  fs.cpSync(path.join(META_SOURCE, 'README.md'), path.join(target, 'README.md'));
  stageLicense(target);

  return target;
}

function stageLicense(target) {
  fs.cpSync(path.join(WORKSPACE, LICENSE_FILE), path.join(target, LICENSE_FILE));
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
  stageLicense(target);

  return target;
}

// A packed tarball named the way npm has to be given it, which is absolutely.
//
// `npm install tarballs/rune-0.1.1.tgz` does not install a file. npm reads any argument
// shaped like `owner/name` as a GitHub repository and goes looking for one over ssh. Only
// a path it recognises as a path — absolute, or starting with `.` — is read as a file, and
// the directory these scripts are given is whatever the caller typed.
function tarballSpec(directory, tarball) {
  return path.resolve(directory, tarball);
}

// The name npm gave the tarball, read out of what `npm pack --json` reported. npm 11
// answers with an array of packed packages and npm 12 with an object keyed by package
// name, so the shape is normalised before the one record is read. Predicting the file
// name instead would put npm's naming rules in this repository, where they would rot.
function tarballName(report) {
  const [packed] = Array.isArray(report) ? report : Object.values(report);

  if (!packed?.filename) {
    throw new Error(`npm pack reported no tarball: ${JSON.stringify(report)}`);
  }
  return packed.filename;
}

function pack(directory, destination) {
  fs.mkdirSync(destination, { recursive: true });
  const report = runNpm(['pack', '--json', '--pack-destination', destination], { cwd: directory });

  return tarballName(JSON.parse(String(report)));
}

// Assemble and pack every package whose binary `binaryFor` can supply, and record what
// came out. The record is what the publish step validates the meta package's pins
// against: "what was built" is then a file, not an assumption.
function packRelease({ outDirectory = DEFAULT_OUTPUT, binaryFor, entries = platforms.PLATFORMS }) {
  const packageVersion = version();
  const tarballs = path.join(outDirectory, 'tarballs');
  fs.rmSync(tarballs, { recursive: true, force: true });

  const packed = entries.map((entry) => ({
    name: entry.package,
    version: packageVersion,
    tarball: pack(
      assemblePlatform(outDirectory, entry, binaryFor(entry), packageVersion),
      tarballs,
    ),
  }));

  const meta = JSON.parse(fs.readFileSync(path.join(META_SOURCE, 'package.json'), 'utf8'));
  packed.push({
    name: meta.name,
    version: packageVersion,
    tarball: pack(assembleMeta(outDirectory), tarballs),
  });

  const record = { version: packageVersion, packed };
  fs.writeFileSync(path.join(tarballs, PACKED), `${JSON.stringify(record, null, 2)}\n`);

  return { ...record, directory: tarballs };
}

// What `just dist` runs: this machine's own release build, packed the way the pipeline
// packs the five it builds.
function main() {
  const entry = platforms.entryFor(process.platform, process.arch);
  if (!entry) {
    throw new Error(`no platform package for ${process.platform} ${process.arch}`);
  }

  const built = path.join(WORKSPACE, 'target', 'release', entry.binary);
  if (!fs.existsSync(built)) {
    throw new Error(`no release binary at ${built} — run \`cargo build --release\` first`);
  }

  const release = packRelease({ binaryFor: () => built, entries: [entry] });

  process.stdout.write(
    `packed ${release.packed.map((one) => one.tarball).join(', ')}\ninto ${release.directory}\n`,
  );
}

if (require.main === module) {
  main();
}

module.exports = {
  LICENSE_FILE,
  PACKED,
  assembleMeta,
  assemblePlatform,
  pack,
  packRelease,
  tarballName,
  tarballSpec,
  version,
};
