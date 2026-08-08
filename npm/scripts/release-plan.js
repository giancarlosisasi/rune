'use strict';

// The decisions a release makes before it publishes anything: what is left to publish,
// whether the meta package's pins describe the packages that were actually built, and in
// what order the two kinds of package go out.
//
// All three are pure functions over state. Written inline in a workflow they could only
// be exercised by running a release and watching what it did; here a release is
// rehearsable on a laptop, and a wrong answer is a failing test rather than a bad
// version on the registry.

const platforms = require('../rune/lib/platforms');

const META = '@giancarlosio/rune';

// Platform packages first, the meta package last.
//
// The meta package's optionalDependencies name exact versions. Published first, it would
// point at platform versions no registry carries yet, and anyone installing during that
// window gets an install that fails or quietly omits the binary.
function publishOrder() {
  return [...platforms.PLATFORMS.map((entry) => entry.package), META];
}

// What is still to publish for `version`, given the packages the registry already carries
// at exactly that version.
//
// A run that published three packages and then failed is completed rather than repeated:
// re-publishing an existing version is an error, and abandoning the release leaves the
// meta package pointing at versions that were never published.
function plan({ version, published = [] }) {
  const already = new Set(published);
  const ordered = publishOrder();

  return {
    version,
    publish: ordered.filter((name) => !already.has(name)),
    skip: ordered.filter((name) => already.has(name)),
  };
}

// Whether the meta package's pins describe exactly the set of packages that was built, at
// exactly the version being released. Returns one message per problem; an empty array is
// the only thing that lets a release continue.
//
// `built` is what the build matrix produced: `[{ name, version }]`.
function validatePins({ manifest, built, version }) {
  const problems = [];

  if (manifest.version !== version) {
    problems.push(`the meta package is at ${manifest.version}, but ${version} is being released`);
  }

  const pinned = new Map(Object.entries(manifest.optionalDependencies ?? {}));
  const produced = new Map(built.map((one) => [one.name, one.version]));

  for (const [name, pin] of pinned) {
    if (!produced.has(name)) {
      problems.push(`${name} is pinned at ${pin} but was not built`);
      continue;
    }
    if (pin !== version) {
      problems.push(`${name} is pinned at ${pin}, not at ${version}`);
    }
  }

  for (const [name, produce] of produced) {
    if (!pinned.has(name)) {
      problems.push(`${name} was built but is not pinned by the meta package`);
      continue;
    }
    if (produce !== version) {
      problems.push(`${name} was built as ${produce}, not as ${version}`);
    }
  }

  return problems;
}

module.exports = { META, plan, publishOrder, validatePins };
