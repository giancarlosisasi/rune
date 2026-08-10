'use strict';

// Reports which copy of the binary ran it, and what started that copy.
//
// The binary a test install presents is node itself, so its own path is the path of the
// install the wrapper chose. The parent tells the two routes apart: reached directly, the
// wrapper that spawned this is the process the test started; reached through a handover,
// there is another wrapper in between.
process.stdout.write(JSON.stringify({ binary: process.execPath, parent: process.ppid }));
