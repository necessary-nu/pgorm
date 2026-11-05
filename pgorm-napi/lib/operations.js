// @ts-check

// What the connection classes share: the addon they call, the abort tokens an
// `AbortSignal` fires, how a row's values become an object, and how a
// callback's transaction ends.
// [spec:pgorm:req:napi.cancellation+2]

import { LifecycleError } from "./errors.js";

/** @type {any} */
export let native;

/**
 * Hand the module the addon its classes call.
 *
 * @param {any} addon
 */
export function installConnections(addon) {
  native = addon;
}

/** Marks a construction from a handle the addon made. */
export const TRUSTED = Symbol("pgorm-napi native handle");

/**
 * @param {unknown} signal
 * @returns {AbortSignal | undefined}
 */
export function signalOf(signal) {
  if (signal === undefined) return undefined;
  if (!(signal instanceof AbortSignal)) throw new TypeError("options.signal is an AbortSignal");
  return signal;
}

/**
 * What a pool, a connection and a transaction do with an operation that
 * finished in the addon after its signal fired, which the caller is told was
 * aborted: nothing, its result being discarded, or, in a transaction, end the
 * transaction with nothing committed, as an abort the addon saw does.
 */
export const LATE = Symbol("pgorm-napi late abort");

/**
 * Run `start` with a native abort token that `signal` fires, rejecting with
 * the signal's reason once it has: an aborted operation's outcome is unknown,
 * and the addon has discarded the connection it ran on.
 *
 * The operation can finish in the addon before the abort reaches it, its
 * result still queued for this thread. The call was aborted first all the
 * same, so it rejects with the signal's reason, after `late` has released
 * what the result holds.
 *
 * @template T
 * @param {AbortSignal | undefined} signal
 * @param {(token: unknown) => Promise<T>} start
 * @param {(result: T) => unknown} [late]
 * @returns {Promise<T>}
 */
export async function abortable(signal, start, late) {
  if (signal === undefined) return await start(undefined);
  signal.throwIfAborted();
  const token = native.abortToken();
  const onAbort = () => native.abort(token);
  signal.addEventListener("abort", onAbort, { once: true });
  let result;
  try {
    result = await start(token);
  } catch (error) {
    if (signal.aborted) throw signal.reason;
    throw error;
  } finally {
    signal.removeEventListener("abort", onAbort);
  }
  if (signal.aborted) {
    await late?.(result);
    throw signal.reason;
  }
  return result;
}

/**
 * Each row an object keyed by column name, in column order. Built with
 * `Object.fromEntries`, which defines its keys, so a column named
 * `__proto__` is a property like any other rather than the object's
 * prototype.
 * [spec:pgorm:req:napi.records]
 *
 * @param {string[]} names
 * @param {unknown[]} values
 */
export function row(names, values) {
  return Object.fromEntries(names.map((name, index) => [name, values[index]]));
}

/**
 * @param {unknown} options
 * @returns {{ tagged: boolean, signal: AbortSignal | undefined }}
 */
export function queryOptions(options) {
  if (typeof options !== "object" || options === null) throw new TypeError("options is an object");
  const { tagged = false, signal } = /** @type {{ tagged?: unknown, signal?: unknown }} */ (options);
  if (typeof tagged !== "boolean") throw new TypeError("options.tagged is a boolean");
  return { tagged, signal: signalOf(signal) };
}

/**
 * Run `use` in the transaction or savepoint `handle` is, which `transaction`
 * wraps: committed (a savepoint released) when `use` resolves, and rolled back
 * when it throws, rejecting with what it threw. A commit that fails ends the
 * transaction, discarding its connection, and rejects with its failure; a
 * ROLLBACK that fails does too, rejecting with a `SuppressedError` over the
 * rollback's failure and what `use` threw.
 * [spec:pgorm:req:napi.transactions+3]
 *
 * @template T
 * @param {unknown} handle
 * @param {import("./connections.js").Transaction} transaction
 * @param {(transaction: import("./connections.js").Transaction) => Promise<T>} use
 * @returns {Promise<T>}
 */
export async function scoped(handle, transaction, use) {
  let result;
  try {
    result = await use(transaction);
  } catch (error) {
    throw await rolledBack(handle, error);
  }
  try {
    await native.transactionFinish(handle, true);
  } catch (error) {
    await native.transactionAbort(handle);
    throw error;
  }
  return result;
}

/**
 * Roll `handle` back after its callback threw `error`, resolving with what
 * `transaction(fn)` rejects with.
 *
 * @param {unknown} handle
 * @param {unknown} error
 * @returns {Promise<unknown>}
 */
async function rolledBack(handle, error) {
  if (!native.transactionClosed(handle)) {
    try {
      await native.transactionFinish(handle, false);
    } catch (failure) {
      await native.transactionAbort(handle);
      // A refusal is the frame given up — a statement or savepoint still held
      // it, or it had just closed — rather than a ROLLBACK the server failed,
      // so the callback's error stands alone.
      if (failure instanceof LifecycleError) return error;
      return new SuppressedError(failure, error, "the transaction's ROLLBACK failed after its callback threw");
    }
  }
  await native.transactionWaitClosed(handle);
  return error;
}
