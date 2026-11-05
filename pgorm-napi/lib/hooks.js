// @ts-check

// A pool's hooks: the program's functions, wrapped for the addon, which calls
// each on this thread as a connection opens or is recycled, and the
// connection each is given.
// [spec:pgorm:req:bindings.pool-hooks]

import { statementArgs } from "./builder.js";
import { native, queryOptions, row, TRUSTED } from "./operations.js";

/** The hook options a pool's shape takes. */
const HOOKS = /** @type {const} */ (["postCreate", "preRecycle", "postRecycle"]);

/**
 * What a hook's function threw or rejected with, as the refusal's text: an
 * error's name and message, or anything else as a string.
 *
 * @param {unknown} error
 * @returns {string}
 */
function refusal(error) {
  if (error instanceof Error) return error.message ? `${error.name}: ${error.message}` : error.name;
  return String(error);
}

/**
 * `hook` as the addon calls it: with the hook's connection and the handle it
 * settles once `hook` has finished, whether it threw, returned or resolved.
 *
 * @param {unknown} hook
 * @param {string} name
 * @returns {((handle: unknown, settle: unknown) => void) | undefined}
 */
function hookCaller(hook, name) {
  if (hook === undefined) return undefined;
  if (typeof hook !== "function") throw new TypeError(`${name} is a function`);
  return (handle, settle) => {
    Promise.resolve()
      .then(() => hook(new HookConnection(handle, TRUSTED)))
      .then(
        () => native.hookSettle(settle, undefined),
        (error) => native.hookSettle(settle, refusal(error)),
      );
  };
}

/**
 * `options` with each hook wrapped for the addon.
 *
 * @param {unknown} options
 * @returns {Record<string, unknown>}
 */
export function hooked(options) {
  if (typeof options !== "object" || options === null) throw new TypeError("options is an object");
  /** @type {Record<string, unknown>} */
  const shaped = { ...options };
  for (const name of HOOKS) shaped[name] = hookCaller(shaped[name], name);
  return shaped;
}

/**
 * The connection a pool's hook is about, while the hook runs: its statements
 * run on that connection, through pgorm's statement cache, as a
 * `Connection`'s do. Once the hook has returned the connection is the pool's,
 * and every statement rejects with a `LifecycleError`.
 */
export class HookConnection {
  /** @type {unknown} */
  #native;

  /**
   * @param {unknown} handle
   * @param {symbol} trusted
   */
  constructor(handle, trusted) {
    if (trusted !== TRUSTED) throw new TypeError("a HookConnection is given to a pool's hook");
    this.#native = handle;
  }

  /** Milliseconds since the connection opened. */
  get age() {
    return native.hookAge(this.#native);
  }

  /** How many times the pool has recycled the connection. */
  get recycleCount() {
    return native.hookRecycleCount(this.#native);
  }

  /** Whether the hook has returned. */
  get closed() {
    return native.hookClosed(this.#native);
  }

  /**
   * @param {"execute" | "all" | "one" | "optional"} terminal
   * @param {unknown} statement
   * @param {unknown} params
   * @param {unknown} options
   * @returns {Promise<any>}
   */
  async #run(terminal, statement, params, options) {
    const [sql, values, rest] = statementArgs(statement, params, options);
    const { tagged } = queryOptions(rest);
    return await native.hookRun(this.#native, terminal, sql, values, tagged);
  }

  /**
   * @param {"all" | "one" | "optional"} terminal
   * @param {unknown} statement
   * @param {unknown} params
   * @param {unknown} options
   * @returns {Promise<Record<string, unknown>[]>}
   */
  async #rows(terminal, statement, params, options) {
    const [names, rows] = await this.#run(terminal, statement, params, options);
    return rows.map((/** @type {unknown[]} */ values) => row(names, values));
  }

  /**
   * Run a statement and resolve with the number of rows it affected.
   *
   * @param {unknown} statement
   * @param {unknown} [params]
   * @param {unknown} [options]
   * @returns {Promise<number>}
   */
  async execute(statement, params, options) {
    return await this.#run("execute", statement, params, options);
  }

  /**
   * Every row.
   *
   * @param {unknown} statement
   * @param {unknown} [params]
   * @param {unknown} [options]
   */
  async query(statement, params, options) {
    return await this.#rows("all", statement, params, options);
  }

  /**
   * Exactly one row; any other count is a `DecodeError`.
   *
   * @param {unknown} statement
   * @param {unknown} [params]
   * @param {unknown} [options]
   */
  async one(statement, params, options) {
    return (await this.#rows("one", statement, params, options))[0];
  }

  /**
   * At most one row, or `null`; more is a `DecodeError`.
   *
   * @param {unknown} statement
   * @param {unknown} [params]
   * @param {unknown} [options]
   */
  async optional(statement, params, options) {
    return (await this.#rows("optional", statement, params, options))[0] ?? null;
  }
}
