// The refusals whose wording is the deliverable, deliberately without `@ts-expect-error`.
//
// The sibling fixture proves each of these is refused. This one is compiled for the text
// the compiler prints, because a refusal a user cannot act on is the defect being repaired:
// `Type 'true' is not assignable to type 'undefined'` names the right field and teaches
// nothing about rune.

import type { Script } from '@gio-labs/rune';

export const interactiveOnAGroup: Script = { parallel: ['dev:server', 'dev:watch'], interactive: true };

export const commandBesideExtends: Script = { command: 'tsc -b', extends: 'build' };

export const successPolicyOnASerialGroup: Script = { serial: ['lint', 'test'], successPolicy: 'first' };

export const dependsOnAGroup: Script = { serial: ['lint', 'test'], dependsOn: ['clean'] };
