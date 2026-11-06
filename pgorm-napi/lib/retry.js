// @ts-check

// The attempts of a retrying transaction: pgorm's `transaction_with_retry`
// opens each, and the callback runs once per attempt until one commits.

import { LifecycleError, PgormError } from "./errors.js";
import { native, TRUSTED } from "./operations.js";

/**
 * Whether `error` is one pgorm runs a transaction again after.
 *
 * @param {unknown} error
 */
function retryable(error) {
  return error instanceof PgormError && error.isRetryable;
}

/**
 * Run `use` in each attempt pgorm opens until one commits or pgorm gives up
 * on it: an attempt whose callback rejects retryably, or whose `COMMIT` fails
 * so, runs again from `BEGIN` while retries remain. Each attempt is a
 * `Transaction` of `parent`'s; the class is passed in because it is
 * connections.js's, which imports this module.
 * [spec:pgorm:req:bindings.transaction-retry]
 *
 * @template T
 * @param {unknown} retrying
 * @param {typeof import("./connections.js").Transaction} Transaction
 * @param {import("./connections.js").Connection} parent
 * @param {(transaction: import("./connections.js").Transaction) => Promise<T>} use
 * @returns {Promise<T>}
 */
export async function retried(retrying, Transaction, parent, use) {
  for (;;) {
    const handle = await native.retryingAttempt(retrying);
    const transaction = new Transaction(handle, parent, TRUSTED);
    /** @type {T} */
    let result;
    try {
      result = await use(transaction);
    } catch (error) {
      if (native.transactionClosed(handle)) {
        native.retryingAbort(retrying);
        throw error;
      }
      let again;
      try {
        again = await native.transactionFinishAttempt(handle, false, retryable(error));
      } catch (failure) {
        native.retryingAbort(retrying);
        if (failure instanceof LifecycleError) throw error;
        throw new SuppressedError(failure, error, "the transaction's ROLLBACK failed after its callback threw");
      }
      if (again) continue;
      throw error;
    }
    let again;
    try {
      again = await native.transactionFinishAttempt(handle, true, false);
    } catch (error) {
      native.retryingAbort(retrying);
      throw error;
    }
    if (!again) return result;
  }
}
