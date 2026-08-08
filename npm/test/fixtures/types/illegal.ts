// Every combination the config format forbids, one per line.
//
// `@ts-expect-error` fails the compile in both directions: the line has to produce an
// error, and if it ever stops producing one the directive is reported as unused. A bare
// "this file does not compile" assertion passes just as happily on a typo in the fixture,
// which is why each case is marked individually and each one is written on a single line.

import type { Script } from '@gio-labs/rune';

// @ts-expect-error a script runs a command of its own or extends another one, never both
export const commandAndExtends: Script = { command: 'tsc -b', extends: 'build' };

// @ts-expect-error a group is serial or parallel
export const serialAndParallel: Script = { serial: ['lint'], parallel: ['test'] };

// @ts-expect-error a group has no command of its own
export const commandAndSerial: Script = { command: 'tsc -b', serial: ['lint'] };

// @ts-expect-error appendArgs adds to an inherited command, so there has to be one to inherit
export const appendArgsWithoutExtends: Script = { command: 'tsc -b', appendArgs: ['--force'] };

// @ts-expect-error a delay between retries means nothing without retries
export const retryDelayWithoutRetries: Script = { command: 'flaky', retryDelay: 500 };

// @ts-expect-error lifecycle options belong to the members of a group, not to the group
export const timeoutOnAGroup: Script = { serial: ['lint', 'test'], timeout: 1000 };

// @ts-expect-error a serial group runs to its policy already: it stops at the first failure
export const successPolicyOnSerial: Script = { serial: ['lint', 'test'], successPolicy: 'first' };

// @ts-expect-error the signal has to be one a platform can actually send
export const unknownKillSignal: Script = { command: 'server', killSignal: 'SIGBANANA' };

// @ts-expect-error a script has to do something
export const nothingAtAll: Script = { description: 'declares nothing to run' };

// @ts-expect-error a misspelled field is a rejection, not a field rune quietly ignores
export const misspelledField: Script = { command: 'tsc -b', descriptoin: 'compile' };
