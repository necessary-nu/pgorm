// @ts-check

// A dedicated listening connection: the channels it listens on, and the
// notifications it receives, pulled one at a time.
// [spec:pgorm:req:bindings.listen+2]

import { LifecycleError } from "./errors.js";
import { abortable, native, signalOf } from "./operations.js";

/** Marks a construction from a handle `pool.listener()` made. */
export const LISTENER = Symbol("pgorm-napi listener handle");

/**
 * The `{ signal }` options a listener's operations take.
 *
 * @param {unknown} options
 * @returns {AbortSignal | undefined}
 */
function signalOption(options) {
  if (typeof options !== "object" || options === null) throw new TypeError("options is an object");
  return signalOf(/** @type {{ signal?: unknown }} */ (options).signal);
}

/**
 * @param {unknown} channel
 * @returns {string}
 */
function channelOf(channel) {
  if (typeof channel !== "string") throw new TypeError("a channel is a string");
  return channel;
}

/**
 * A connection of its own, opened by `pool.listener()` with the pool's
 * configuration, receiving the notifications sent on the channels it listens
 * on. It is an async iterator of them, one per `next()`, in the order the
 * server sent them; nothing reconnects it. Its iteration ends once it closes:
 * by `close()`, which ends a waiting pull as done; by a `for await` loop
 * leaving early; by its pool closing, which a waiting pull rejects with a
 * `LifecycleError`; or by the failure it ends with — its connection lost, or
 * a notification arriving while `capacity` wait unread — which the pull that
 * meets it rejects with.
 *
 * @implements {AsyncIterableIterator<import("./index.d.ts").Notification>}
 */
export class Listener {
  /** @type {unknown} */
  #native;
  #done = false;
  #pulling = false;
  /**
   * The notification a pull received after its abort, kept for the next
   * pull: the aborted one lost nothing.
   *
   * @type {readonly [string, string, number] | undefined}
   */
  #kept;

  /**
   * @param {unknown} handle
   * @param {symbol} trusted
   */
  constructor(handle, trusted) {
    if (trusted !== LISTENER) throw new TypeError("a Listener comes from pool.listener()");
    this.#native = handle;
  }

  /** @returns {boolean} */
  get closed() {
    return this.#done || native.listenerClosed(this.#native);
  }

  /**
   * @param {"listen" | "unlisten" | "unlistenAll"} verb
   * @param {string} channel
   * @param {unknown} options
   * @returns {Promise<void>}
   */
  async #send(verb, channel, options) {
    const signal = signalOption(options);
    await abortable(
      signal,
      (token) => native.listenerSend(this.#native, verb, channel, token),
      () => this.#finish(),
    );
  }

  /**
   * Receive the notifications sent on `channel`, its name exact: case and
   * punctuation are kept. Aborted, the listener closes, since the server may
   * have listened.
   *
   * @param {string} channel
   * @param {{ signal?: AbortSignal }} [options]
   * @returns {Promise<void>}
   */
  async listen(channel, options = {}) {
    await this.#send("listen", channelOf(channel), options);
  }

  /**
   * Stop receiving the notifications sent on `channel`.
   *
   * @param {string} channel
   * @param {{ signal?: AbortSignal }} [options]
   * @returns {Promise<void>}
   */
  async unlisten(channel, options = {}) {
    await this.#send("unlisten", channelOf(channel), options);
  }

  /**
   * Stop receiving notifications on every channel.
   *
   * @param {{ signal?: AbortSignal }} [options]
   * @returns {Promise<void>}
   */
  async unlistenAll(options = {}) {
    await this.#send("unlistenAll", "", options);
  }

  /**
   * The next notification. An aborted pull leaves the listener open: the
   * notification it waited for is the next pull's.
   *
   * @param {{ signal?: AbortSignal }} [options]
   * @returns {Promise<IteratorResult<import("./index.d.ts").Notification, undefined>>}
   */
  async next(options = {}) {
    if (this.closed) return { done: true, value: undefined };
    if (this.#pulling) throw new LifecycleError("the listener is busy: one notification is pulled at a time");
    const signal = signalOption(options ?? {});
    this.#pulling = true;
    try {
      signal?.throwIfAborted();
      const kept = this.#kept;
      this.#kept = undefined;
      /** @type {readonly [string, string, number] | null} */
      const item = kept !== undefined ? kept : await abortable(
        signal,
        (token) => native.listenerNext(this.#native, token),
        (/** @type {readonly [string, string, number] | null} */ received) => {
          if (received === null) this.#done = true;
          else this.#kept = received;
        },
      );
      if (item === null) {
        this.#done = true;
        return { done: true, value: undefined };
      }
      const [channel, payload, processId] = item;
      return { done: false, value: Object.freeze({ channel, payload, processId }) };
    } catch (error) {
      if (!signal?.aborted) await this.#finish();
      throw error;
    } finally {
      this.#pulling = false;
    }
  }

  /**
   * Close the listener, as a `for await` loop does when it leaves early.
   *
   * @returns {Promise<IteratorResult<import("./index.d.ts").Notification, undefined>>}
   */
  async return() {
    await this.#finish();
    return { done: true, value: undefined };
  }

  /**
   * Close the listener's connection, ending a pull in flight as done, and
   * resolve once it is released.
   *
   * @returns {Promise<void>}
   */
  async close() {
    await this.#finish();
  }

  async #finish() {
    this.#done = true;
    this.#kept = undefined;
    await native.listenerClose(this.#native);
  }

  [Symbol.asyncIterator]() {
    return this;
  }

  [Symbol.asyncDispose]() {
    return this.close();
  }
}

/**
 * The capacity option of `pool.listener()`: a positive whole number of
 * notifications held unread.
 *
 * @param {unknown} options
 * @returns {{ capacity: number, signal: AbortSignal | undefined }}
 */
export function listenerOptions(options) {
  const signal = signalOption(options);
  const { capacity = 1024 } = /** @type {{ capacity?: unknown }} */ (options);
  if (typeof capacity !== "number") throw new TypeError("options.capacity is a number");
  if (!Number.isSafeInteger(capacity) || capacity < 1) {
    throw new RangeError("options.capacity is a positive whole number");
  }
  return { capacity, signal };
}

/**
 * The job `notify` runs: `SELECT pg_notify($1, $2)`, both bound.
 *
 * @param {unknown} channel
 * @param {unknown} payload
 * @returns {unknown}
 */
export function notifyJob(channel, payload) {
  if (typeof payload !== "string") throw new TypeError("a payload is a string");
  return native.notifyJob(channelOf(channel), payload);
}
