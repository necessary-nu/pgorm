// @ts-check

// The data side of a COPY FROM shared by its byte and row forms: pulling a
// source as the COPY takes its data, queueing pieces within the addon's
// budget, and abandoning the COPY when the source fails or the COPY is cut
// off.
// [spec:pgorm:req:bindings.copy+1]

import { native } from "./operations.js";

/**
 * How large a piece of a chunk crosses into the addon at once: a chunk of a
 * hundred megabytes is copied and queued a piece at a time, each within the
 * addon's budget of queued bytes.
 */
const PIECE = 64 * 1024;

/**
 * How a source's pull watches the COPY it feeds: the error the COPY was cut
 * off with once the connection, pool or transaction under it closed, the
 * pull waiting to hear it — one slot, so watching costs nothing per chunk —
 * and whether the last pull failed by the source's own error, which leaves
 * nothing to close.
 *
 * @typedef {{ failed: boolean, failure: unknown, wake: ((error: unknown) => void) | undefined, sourceFailed: boolean }} Watch
 */

/**
 * Watch `ended`, which rejects once a COPY is cut off.
 *
 * @param {Promise<void>} ended
 * @returns {Watch}
 */
export function watching(ended) {
  /** @type {Watch} */
  const watch = { failed: false, failure: undefined, wake: undefined, sourceFailed: false };
  ended.then(undefined, (error) => {
    watch.failed = true;
    watch.failure = error;
    watch.wake?.(error);
  });
  return watch;
}

/**
 * The iterator's next step, or a rejection once the signal fires or the COPY
 * is cut off: a source that never yields again does not hold open a COPY that
 * has ended.
 *
 * @param {Iterator<unknown> | AsyncIterator<unknown>} iterator
 * @param {AbortSignal | undefined} signal
 * @param {Watch} watch
 * @returns {Promise<IteratorResult<unknown>>}
 */
export function step(iterator, signal, watch) {
  signal?.throwIfAborted();
  if (watch.failed) throw watch.failure;
  return new Promise((resolve, reject) => {
    const onAbort = () => reject(signal?.reason);
    signal?.addEventListener("abort", onAbort, { once: true });
    watch.wake = reject;
    const settled = () => {
      signal?.removeEventListener("abort", onAbort);
      watch.wake = undefined;
    };
    try {
      Promise.resolve(iterator.next()).then(
        (result) => {
          settled();
          resolve(result);
        },
        (error) => {
          settled();
          watch.sourceFailed = true;
          reject(error);
        },
      );
    } catch (error) {
      settled();
      watch.sourceFailed = true;
      reject(error);
    }
  });
}

/**
 * Queue `chunk` for the COPY a piece at a time, waiting on the addon's budget
 * while what it has queued is not yet taken.
 *
 * @param {unknown} handle
 * @param {Uint8Array} chunk
 */
export async function write(handle, chunk) {
  for (let at = 0; at < chunk.length; at += PIECE) {
    const piece = chunk.subarray(at, at + PIECE);
    while (!native.copyInWrite(handle, piece)) await native.copyInReady(handle, piece.length);
  }
}

/**
 * Feed a COPY FROM the addon has opened from `open`'s iterator, pulling the
 * next chunk only once the last is queued, and finish it, resolving with the
 * rows it wrote. If the source throws, yields something that is not a
 * `Uint8Array`, or the signal fires, the COPY is abandoned — the server
 * writes nothing — and the promise rejects with that error; an iterator left
 * mid-way is closed.
 *
 * @param {unknown} handle
 * @param {() => Iterator<unknown> | AsyncIterator<unknown>} open
 * @param {AbortSignal | undefined} signal
 * @returns {Promise<number>}
 */
export async function pump(handle, open, signal) {
  /** @type {Iterator<unknown> | AsyncIterator<unknown> | undefined} */
  let iterator;
  let pulling = false;
  const watch = watching(native.copyInEnded(handle));
  try {
    iterator = open();
    for (;;) {
      pulling = true;
      const next = await step(iterator, signal, watch);
      pulling = false;
      if (next.done) break;
      const chunk = next.value;
      if (!(chunk instanceof Uint8Array)) {
        throw new TypeError(`a COPY's source yields Uint8Array chunks, not ${describe(chunk)}`);
      }
      if (chunk.length > 0) await write(handle, chunk);
    }
  } catch (error) {
    await native.copyInAbort(handle);
    if (iterator !== undefined && !pulling) await close(iterator);
    else if (iterator !== undefined && !watch.sourceFailed) void close(iterator);
    throw error;
  }
  return await native.copyInFinish(handle);
}

/**
 * Close an iterator left mid-way, as `for await` does when its body throws.
 * Its own failure to close is not the COPY's.
 *
 * @param {Iterator<unknown> | AsyncIterator<unknown>} iterator
 */
export async function close(iterator) {
  try {
    await iterator.return?.();
  } catch {
    // The COPY's own error stands.
  }
}

/** @param {unknown} value */
export function describe(value) {
  if (value === null) return "null";
  if (typeof value === "object") return value.constructor?.name ?? "an object";
  return typeof value;
}
