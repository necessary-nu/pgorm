// @ts-check

// Reads the addon builds, run on whichever database a caller passes: a
// built statement's rows as the addon gives them, a registered entity's job,
// and a stream of rows the addon decodes itself.

import { statementArgs } from "./builder.js";
import { Connection, connectionHandle, Pool, poolHandle, Queryable, Transaction, transactionHandle } from "./connections.js";
import { abortable, JOB, LATE, native, queryOptions, RUN, TRUSTED } from "./operations.js";
import { RowStream } from "./streams.js";

/**
 * Run a built statement and resolve with its result as the addon gives it,
 * `[names, rows, kinds]` — each row an array of values in column order, and
 * each column's kind — for a model to decode against its declaration.
 * [spec:pgorm:req:bindings.model-records+1]
 *
 * @param {unknown} db
 * @param {"execute" | "all" | "one" | "optional"} terminal
 * @param {unknown} statement A builder's, or SQL text a raw read holds.
 * @param {unknown} options
 * @returns {Promise<any>}
 */
export async function runStatement(db, terminal, statement, options) {
  if (!(db instanceof Queryable)) {
    throw new TypeError("a model's statement runs on a Pool, a Connection or a Transaction");
  }
  const [handle, values, rest] = statementArgs(statement, options, undefined);
  const { signal } = queryOptions(rest);
  return await abortable(signal, (token) => db[RUN](terminal, handle, values, false, token), () => db[LATE]());
}

/**
 * The rows of `statement`, the native half of a `Select`, as a stream on a
 * pool's connection of its own, on a connection, or, where `transactions`
 * says so, in a transaction, each row read by `decoder`: a disjoint read's
 * codec, or a registered read's selector. `what` names the read in the
 * refusal of any other database.
 * [spec:pgorm:req:bindings.disjoint+2]
 * [spec:pgorm:req:bindings.streams+1]
 *
 * @param {unknown} db
 * @param {unknown} statement
 * @param {unknown} decoder
 * @param {unknown} options
 * @param {string} what
 * @param {boolean} [transactions]
 * @returns {RowStream}
 */
export function decodedStream(db, statement, decoder, options, what, transactions = false) {
  if (db instanceof Pool) {
    return new RowStream(async (token) => {
      const connection = new Connection(await native.poolAcquire(poolHandle(db), token), TRUSTED);
      try {
        const handle = await native.connectionStream(connectionHandle(connection), statement, [], token, decoder);
        return [handle, () => connection.close()];
      } catch (error) {
        await connection.close();
        throw error;
      }
    }, options ?? {}, TRUSTED);
  }
  if (db instanceof Connection) {
    return new RowStream(
      async (token) => [await native.connectionStream(connectionHandle(db), statement, [], token, decoder), undefined],
      options ?? {},
      TRUSTED,
    );
  }
  if (transactions && db instanceof Transaction) {
    return new RowStream(
      async (token) => [await native.transactionStream(transactionHandle(db), statement, [], token, decoder), undefined],
      options ?? {},
      TRUSTED,
    );
  }
  throw new TypeError(`${what}'s rows stream from a Pool${transactions ? ", a Connection or a Transaction" : " or a Connection"}`);
}

/**
 * Run a registered entity's job — one of pgorm's own terminals, built by the
 * addon — on a pool's connection, a connection or a transaction, with the
 * operation a statement gets around it: the busy refusal, the abort, the
 * discard of a connection an abort left in an unknown state.
 * [spec:pgorm:req:bindings.entities+6]
 * [spec:pgorm:req:bindings.registered-pools]
 *
 * @param {unknown} db
 * @param {unknown} job
 * @param {unknown} options
 * @returns {Promise<any>}
 */
export async function runJob(db, job, options) {
  if (!(db instanceof Pool || db instanceof Connection || db instanceof Transaction)) {
    throw new TypeError("a registered entity runs on a Pool, a Connection or a Transaction");
  }
  const { signal } = queryOptions(options ?? {});
  return await abortable(signal, (token) => db[JOB](job, token), () => db[LATE]());
}
