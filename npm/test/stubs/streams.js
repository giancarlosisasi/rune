'use strict';

// Proves it was handed the wrapper's own streams: it answers on standard output what it
// read from standard input, and writes a separate line to standard error.
let read = '';

process.stdin.setEncoding('utf8');
process.stdin.on('data', (chunk) => {
  read += chunk;
});
process.stdin.on('end', () => {
  process.stdout.write(`heard: ${read.trim()}\n`);
  process.stderr.write('this went to standard error\n');
});
