'use strict';

// Running npm from a script, on every operating system.
//
// On Windows npm is a `.cmd`, which node refuses to spawn on its own. It goes through
// the command interpreter instead — the same `/d /s /c` form rune uses for its own
// children — so the arguments are quoted here rather than handed to a shell unescaped.

const { execFileSync } = require('node:child_process');

const WINDOWS = process.platform === 'win32';

function runNpm(args, options = {}) {
  const settings = { stdio: 'pipe', ...options };

  if (!WINDOWS) {
    return execFileSync('npm', args, settings);
  }

  const interpreter = process.env.ComSpec || 'cmd.exe';
  const line = ['npm', ...args.map(quote)].join(' ');

  return execFileSync(interpreter, ['/d', '/s', '/c', line], { ...settings, windowsVerbatimArguments: true });
}

function quote(argument) {
  return /[\s&|<>^()"]/.test(argument) ? `"${argument}"` : argument;
}

module.exports = { runNpm };
