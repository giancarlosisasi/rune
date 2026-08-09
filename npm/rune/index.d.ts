/**
 * The shape of a rune config, as of the version that publishes this file.
 *
 * What typechecks here must load in rune. The two are kept honest by a fixture that is
 * compiled by `tsc` and then loaded by the binary, so an illegal combination is a type
 * error and a runtime error alike.
 *
 * This file is the whole authoring surface: a config that imports from here needs no
 * declaration file of its own. The fixture compiles with an empty `types` option to keep
 * that true.
 */

/** A command that differs by operating system. `default` covers the rest. */
export type PerOsCommand = {
  default: string;
  win32?: string;
  darwin?: string;
  linux?: string;
};

/** What a script may be asked to end with, before a kill follows. */
export type KillSignal = 'SIGHUP' | 'SIGINT' | 'SIGQUIT' | 'SIGTERM' | 'SIGKILL';

/** Marks fields that belong to another variant, so mixing two of them is a type error. */
type Forbid<Keys extends string> = { [Key in Keys]?: never };

interface Common {
  /** One line, shown by `rune list`. */
  description?: string;
  /**
   * Where the command runs. A relative path resolves against this config file, the same
   * way `envFile` does. Omit it and the command runs in the package it was invoked from.
   */
  cwd?: string;
  /** Variables added to the environment. These override an `envFile`. */
  env?: Record<string, string>;
  /** A dotenv file to load. It never overrides a variable the environment already has. */
  envFile?: string;
}

/** Retries and the wait between them. A delay without retries is meaningless, so it is refused. */
type Retrying =
  | { retries: number; retryDelay?: number | 'exponential' }
  | { retries?: never; retryDelay?: never };

/** Options for a script that runs a process. Every duration is in milliseconds. */
type Lifecycle = Retrying & {
  timeout?: number;
  killSignal?: KillSignal;
  killTimeout?: number;
  /** Keeps the terminal: inherited streams, and no process group of its own. */
  interactive?: boolean;
};

type GroupOnly = 'serial' | 'parallel' | 'continueOnError' | 'successPolicy';
type RunnerOnly =
  | 'command'
  | 'extends'
  | 'appendArgs'
  | 'dependsOn'
  | 'timeout'
  | 'retries'
  | 'retryDelay'
  | 'killSignal'
  | 'killTimeout'
  | 'interactive';

/** A script that runs a command of its own. */
export type CommandScript = Common &
  Lifecycle &
  Forbid<GroupOnly | 'extends' | 'appendArgs'> & {
    command: string | PerOsCommand;
    /** Scripts that run, in order, before this one. */
    dependsOn?: string[];
  };

/** A script that takes another one and adds to it. */
export type ExtendsScript = Common &
  Lifecycle &
  Forbid<GroupOnly | 'command'> & {
    extends: string;
    /** Appended to the inherited command, after everything it already carries. */
    appendArgs?: string[];
    dependsOn?: string[];
  };

/** Scripts run one at a time, in the order written. */
export type SerialScript = Common &
  Forbid<RunnerOnly | 'parallel' | 'successPolicy'> & {
    serial: string[];
    /** Runs the rest after a member fails. The group still fails. */
    continueOnError?: boolean;
  };

/** Scripts run at the same time, their output attributed per script. */
export type ParallelScript = Common &
  Forbid<RunnerOnly | 'serial'> & {
    parallel: string[];
    continueOnError?: boolean;
    /** Which members have to succeed for the group to succeed. Defaults to `all`. */
    successPolicy?: 'all' | 'first' | 'last';
  };

export type Script = CommandScript | ExtendsScript | SerialScript | ParallelScript;

export interface RuneConfig {
  scripts: Record<string, Script>;
}

/**
 * Types a config without naming the type.
 *
 * ```ts
 * import { defineConfig } from '@gio-labs/rune';
 *
 * export default defineConfig({
 *   scripts: {
 *     build: { command: 'tsc --build', description: 'Compile every package' },
 *   },
 * });
 * ```
 */
export declare function defineConfig(config: RuneConfig): RuneConfig;

/** What a config is allowed to know about the machine it is being read on. */
export interface Rune {
  /** Read access to the process environment. Reading a variable makes the cached result
   *  depend on it; a variable never read never invalidates anything. */
  readonly env: Readonly<Record<string, string | undefined>>;
  /** The running operating system, matching Node's `process.platform`. */
  readonly platform: 'win32' | 'darwin' | 'linux';
  /** Whether `CI` is set to anything other than empty, `0` or `false`. */
  readonly isCI: boolean;
}

/**
 * The environment, frozen. Import it wherever you branch on the machine — the entry
 * config or any file it imports.
 *
 * ```ts
 * import { defineConfig, rune } from '@gio-labs/rune';
 *
 * export default defineConfig({
 *   scripts: {
 *     test: { command: rune.isCI ? 'vitest --run' : 'vitest' },
 *   },
 * });
 * ```
 */
export declare const rune: Rune;
