'use strict';

// The message a user gets when there is no binary to run.
//
// At that moment this text is their only source of information, so it names the cause it
// can prove and gives the command that repairs it. It is a pure function of what was
// observed — every fact arrives as an argument, which is what makes the exact wording
// testable.

const platforms = require('./platforms');

const ISSUES = 'https://github.com/giancarlosisasi/rune/issues';

function diagnose(context) {
  switch (context.kind) {
    case 'override-missing':
      return overrideMissing(context);
    case 'unsupported':
      return unsupported(context);
    default:
      return missing(context);
  }
}

function overrideMissing({ variable, path: named }) {
  return [
    `${variable} names a binary that is not there:`,
    '',
    `  ${named}`,
    '',
    'rune will not fall back to the installed release while that variable is set. A stale',
    'path has to be reported, not worked around: unset it, or point it at a binary that exists.',
    '',
  ].join('\n');
}

function unsupported({ platform, arch }) {
  return [
    `rune has no binary for ${platform} ${arch}.`,
    '',
    'supported platforms:',
    ...platforms.supported().map((name) => `  ${name}`),
    '',
    `if this one should be here too, say so at ${ISSUES}`,
    '',
  ].join('\n');
}

function missing(context) {
  const { platform, arch, package: name } = context;
  const lines = [`rune could not find its binary for ${platform} ${arch}.`, ''];

  lines.push(...cause(context, name));
  lines.push('', 'repair it with:', '', `  ${repairCommand(context)}`, '');

  if (context.tried?.length) {
    lines.push('it looked for:', ...context.tried.map((specifier) => `  ${specifier}`), '');
  }

  return lines.join('\n');
}

// The branches are ordered by how much they explain. A lockfile that omits the package
// is the single most common cause and the only one with a citation behind it.
function cause(context, name) {
  if (context.lockfile && !context.lockfile.mentionsPackage) {
    return [
      `\`${name}\` is missing from this install, and from ${context.lockfile.path}.`,
      '',
      'package managers have a long-standing defect that drops the optional dependencies of',
      'other platforms from a lockfile written on one machine (npm/cli#4828). An install made',
      'from that lockfile then has no binary for this platform to install.',
    ];
  }

  const elsewhere = (context.foreign ?? []).filter((installed) => installed !== name);
  if (elsewhere.length) {
    return [
      `\`${name}\` is missing, but the binary of another platform is installed:`,
      ...elsewhere.map((installed) => `  ${installed}`),
      '',
      'one node_modules directory is being shared by two systems. A folder mounted into WSL or',
      'into a container, or a dependency directory copied into an image built somewhere else,',
      'both end this way. Install from the machine that runs rune.',
    ];
  }

  return [
    `\`${name}\` ships with rune as an optional dependency, and is not installed.`,
    '',
    'an interrupted install, or one made with optional dependencies turned off, leaves exactly',
    'this behind.',
  ];
}

function repairCommand({ packageManager, hostPlatform }) {
  const name = packageManager?.name;

  if (name === 'pnpm' || name === 'bun') {
    return `${name} install --force`;
  }

  if (name === 'yarn') {
    return packageManager.major === 1
      ? 'yarn install --check-files'
      : `${remove(hostPlatform, ['node_modules'])} && yarn install`;
  }

  return `${remove(hostPlatform, ['node_modules', 'package-lock.json'])} && npm install`;
}

function remove(hostPlatform, targets) {
  return hostPlatform === 'win32'
    ? `Remove-Item -Recurse -Force ${targets.join(', ')}`
    : `rm -rf ${targets.join(' ')}`;
}

module.exports = { diagnose };
