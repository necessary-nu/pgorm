// @ts-check

// Several pools against one server, one per tag.
// [spec:pgorm:req:bindings.multi-pool]

import { Pool, POOL_SHAPE_OPTIONS, SHARED_POOL_OPTIONS } from "./connections.js";
import { hooked } from "./hooks.js";
import { knownOptions, native, TRUSTED } from "./operations.js";

/**
 * Several pools against one server, one per own property of `pools`, each
 * tagged with its key and shaped by its value's options, sharing `options`:
 * the connection string, TLS, the recycling method and the statement cache.
 * Building one sends nothing; nothing balances or falls back across them.
 */
export class MultiPool {
  /** @type {Map<string, Pool>} */
  #pools = new Map();

  /**
   * @param {string} dsn
   * @param {Record<string, import("./index.d.ts").PoolShapeOptions>} pools
   * @param {import("./index.d.ts").SharedPoolOptions} [options]
   */
  constructor(dsn, pools, options = {}) {
    if (typeof dsn !== "string") throw new TypeError("dsn is a connection string");
    if (typeof pools !== "object" || pools === null) throw new TypeError("pools maps each pool's tag to its options");
    if (typeof options !== "object" || options === null) throw new TypeError("options is an object");
    knownOptions(options, SHARED_POOL_OPTIONS);
    /** @type {Record<string, unknown>} */
    const shapes = {};
    for (const [tag, shape] of Object.entries(pools)) {
      if (typeof shape !== "object" || shape === null) throw new TypeError(`pool ${JSON.stringify(tag)}'s options are an object`);
      knownOptions(shape, POOL_SHAPE_OPTIONS);
      shapes[tag] = hooked(shape);
    }
    for (const [tag, handle] of native.poolsNew(dsn, shapes, options)) {
      this.#pools.set(tag, new Pool(TRUSTED, handle));
    }
  }

  /**
   * The pool tagged `tag`, or `null`.
   *
   * @param {string} tag
   * @returns {Pool | null}
   */
  get(tag) {
    return this.#pools.get(tag) ?? null;
  }

  /**
   * Every pool with its tag, in the tags' order.
   *
   * @returns {[string, Pool][]}
   */
  entries() {
    return [...this.#pools.entries()];
  }

  /**
   * Each pool's status, by tag.
   *
   * @returns {Record<string, import("./index.d.ts").PoolStatus>}
   */
  status() {
    return Object.fromEntries([...this.#pools].map(([tag, pool]) => [tag, pool.status()]));
  }

  /** Whether every pool is closed. */
  get closed() {
    return [...this.#pools.values()].every((pool) => pool.closed);
  }

  /**
   * Close every pool.
   *
   * @returns {Promise<void>}
   */
  async close() {
    await Promise.all([...this.#pools.values()].map((pool) => pool.close()));
  }

  [Symbol.asyncDispose]() {
    return this.close();
  }
}
