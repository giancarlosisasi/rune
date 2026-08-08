// One definition, at the root, for every package under it.
//
// The commands take no arguments and touch no files on purpose: this fixture answers
// "did the released binary install, find this file and run something", and a command
// that needed a particular working directory or a particular shell quoting rule would
// answer a different question on each operating system.

export default {
  scripts: {
    greet: {
      command: "node -v",
      description: "print the node version",
    },
    // A group, so the release is proven to carry more than a single spawn.
    check: {
      serial: ["greet"],
      description: "everything this fixture has to pass",
    },
  },
};
