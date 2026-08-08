'use strict';

// The one place a platform is named.
//
// Names use npm's own vocabulary for the operating system and the architecture, so a
// package name never has to be translated into an `os` or a `cpu` value, and nothing
// here needs a lookup table to talk to a package manager.
//
// `target` is the Rust triple the binary is built with. Two entries share one triple:
// the Windows arm64 package ships the x64 binary and runs under emulation, so six
// packages come out of five builds.
//
// "musl" is deliberately absent from every name. Static linking is how the Linux binary
// is built, not what it is for; encoding it would make shipping a glibc build later a
// breaking rename.

const SCOPE = '@gio-labs';
const REPOSITORY = 'https://github.com/giancarlosisasi/rune';

// Where the executable sits inside a platform package. The wrapper resolves this path;
// no platform package declares a `bin` entry of its own.
const BINARY_DIRECTORY = 'bin';

const PLATFORMS = [
  { os: 'win32', cpu: 'x64', target: 'x86_64-pc-windows-msvc', runner: 'windows-latest' },
  { os: 'win32', cpu: 'arm64', target: 'x86_64-pc-windows-msvc', runner: 'windows-latest' },
  { os: 'darwin', cpu: 'x64', target: 'x86_64-apple-darwin', runner: 'macos-latest' },
  { os: 'darwin', cpu: 'arm64', target: 'aarch64-apple-darwin', runner: 'macos-latest' },
  { os: 'linux', cpu: 'x64', target: 'x86_64-unknown-linux-musl', runner: 'ubuntu-latest' },
  { os: 'linux', cpu: 'arm64', target: 'aarch64-unknown-linux-musl', runner: 'ubuntu-latest' },
].map((entry) =>
  Object.freeze({
    ...entry,
    package: `${SCOPE}/rune-${entry.os}-${entry.cpu}`,
    binary: entry.os === 'win32' ? 'rune.exe' : 'rune',
  }),
);

Object.freeze(PLATFORMS);

// What `require.resolve` is given to find the executable of one platform package.
function specifier(entry) {
  return `${entry.package}/${BINARY_DIRECTORY}/${entry.binary}`;
}

function entryFor(os, cpu) {
  return PLATFORMS.find((entry) => entry.os === os && entry.cpu === cpu);
}

// The x64 package of the same operating system, which an arm64 host can run under
// emulation. This is the second of arm64's two mechanisms: the first is the x64 binary
// shipped inside the arm64 package, and this one catches the arm64 package being absent.
function emulatedFallbackFor(os, cpu) {
  return cpu === 'arm64' ? entryFor(os, 'x64') : undefined;
}

function manifest(entry, version) {
  return {
    name: entry.package,
    version,
    description: `The rune binary for ${entry.os} ${entry.cpu}.`,
    homepage: REPOSITORY,
    repository: { type: 'git', url: `git+${REPOSITORY}.git` },
    license: 'MIT',
    os: [entry.os],
    cpu: [entry.cpu],
    // Yarn Plug'n'Play keeps package contents inside a zip. The payload here is an
    // executable, which has to be a real file on disk.
    preferUnplugged: true,
    publishConfig: { access: 'public' },
  };
}

// Every pin, at exactly one version. Nothing here is ever hand-edited.
function optionalDependencies(version) {
  return Object.fromEntries(PLATFORMS.map((entry) => [entry.package, version]));
}

// zig cross-compiles the QuickJS C source rquickjs bundles without the linker patches a
// plain rustup target needs against musl. The native builds have no such problem, and
// paying for zig there would only slow them down.
function builderFor(target) {
  return target.endsWith('-linux-musl') ? 'cargo-zigbuild' : 'cargo';
}

// One build per target triple, carrying the packages that triple's binary fills. This is
// the release workflow's build matrix, so the workflow never names a platform itself.
function releaseMatrix() {
  const builds = new Map();

  for (const entry of PLATFORMS) {
    const build = builds.get(entry.target) ?? {
      target: entry.target,
      runner: entry.runner,
      builder: builderFor(entry.target),
      binary: entry.binary,
      packages: [],
    };
    build.packages.push(entry.package);
    builds.set(entry.target, build);
  }

  return [...builds.values()];
}

// The supported matrix as a user reads it, for the unsupported-platform diagnostic.
function supported() {
  return PLATFORMS.map((entry) => `${entry.os} ${entry.cpu}`);
}

module.exports = {
  BINARY_DIRECTORY,
  PLATFORMS,
  emulatedFallbackFor,
  entryFor,
  manifest,
  optionalDependencies,
  releaseMatrix,
  specifier,
  supported,
};
