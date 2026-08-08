'use strict';

// Dies from the signal it was named, so the wrapper has a signal death to pass on.
process.kill(process.pid, process.argv[2]);
setTimeout(() => {}, 1000);
