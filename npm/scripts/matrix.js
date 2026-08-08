'use strict';

// Every matrix the release workflow runs.
//
// No workflow file names a platform, a target triple or a runner: it asks here, and here
// asks the platform table. A seventh package is then one entry in one file, and the
// pipeline grows a build without being edited.

const platforms = require('../rune/lib/platforms');

// docker's architecture vocabulary is neither npm's nor Rust's. This is the only place
// the three of them meet.
const DOCKER_ARCH = { x64: 'amd64', arm64: 'arm64' };

// Both C libraries a Linux user can have. Static linking is what makes one binary enough
// for both, and running it in each is what turns that from a decision into a guarantee.
const LINUX_BASES = [
  { libc: 'musl', image: 'alpine:3.21' },
  { libc: 'glibc', image: 'debian:bookworm-slim' },
];

const MATRICES = {
  // One build per target triple. Six packages come out of five builds, because the
  // Windows arm64 package ships the x64 binary.
  build: () => platforms.releaseMatrix(),

  // The Linux binaries, each under both C libraries.
  libc: () =>
    platforms.PLATFORMS.filter((entry) => entry.os === 'linux').flatMap((entry) =>
      LINUX_BASES.map((base) => ({
        ...base,
        target: entry.target,
        platform: `linux/${DOCKER_ARCH[entry.cpu]}`,
      })),
    ),

  // One runner per operating system rune ships for.
  install: () => [...new Set(platforms.PLATFORMS.map((entry) => entry.runner))],
};

function matrix(name) {
  const build = MATRICES[name];
  if (!build) {
    throw new Error(`no matrix called ${name}; there is ${Object.keys(MATRICES).join(', ')}`);
  }
  return build();
}

if (require.main === module) {
  const [name] = process.argv.slice(2);
  process.stdout.write(`${JSON.stringify(matrix(name))}\n`);
}

module.exports = { matrix };
