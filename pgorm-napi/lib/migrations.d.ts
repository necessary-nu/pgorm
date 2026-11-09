/**
 * pgorm-migration's runner: a program's migrations applied in order, each
 * once, and recorded in the ledger, as Rust's `Migrations` applies them.
 *
 * @module
 */

import type { OperationOptions, Pool, Queryable, Transaction } from "./index.d.ts";

// [spec:pgorm:req:bindings.migrations]
/**
 * A migration written in JavaScript: a `name`, the migration's identity in
 * the ledger, and an `up` the runner awaits in the batch's transaction. An
 * optional `checksum` is recorded and checked as Rust's
 * `MigrationTrait::checksum` is, and an optional `commitsAfter` commits the
 * batch once the migration has run. All three are read once, as the
 * migrator is built.
 */
export interface MigrationTrait {
  readonly name: string;
  up(tx: Transaction): Promise<void>;
  readonly checksum?: string | null;
  readonly commitsAfter?: boolean;
}

/** A migration's name and whether the ledger records it applied. */
export interface Migration {
  readonly name: string;
  readonly status: "pending" | "applied";
}

/** A migration whose whole body is one SQL script, pgorm-migration's `SqlMigration`. */
export declare class SqlMigration {
  private constructor();
  /** The migration's name, its file's stem when it was read from one. */
  readonly name: string;
  /** The script `up` runs, whole, in the batch's transaction. */
  readonly sql: string;
  /** The SHA-256 digest of the script, which the ledger records. */
  readonly checksum: string;
  /** Whether the script opens with `-- pgorm: commits-after`. */
  readonly commitsAfter: boolean;
  /**
   * The migration the `.sql` file at `path` holds, named for its stem, its
   * CRLF line endings read as LF. A path naming no file stem is a
   * `ConstructionError`, and any other failure to read an `Error`.
   */
  static read(path: string | URL): SqlMigration;
  /** Every `.sql` file directly inside `directory`, each read as `read` reads one, in file-name order. */
  static readDir(directory: string | URL): SqlMigration[];
}

/** The migration `name` running `sql`. */
export declare function sqlMigration(name: string, sql: string): SqlMigration;

/** Where a migrator's ledger lives, and the channel its batches notify. */
export interface MigrationsOptions {
  /** The ledger's table name, `pgorm_migrations` unless given. */
  readonly migrationTableName?: string | null;
  /** The ledger's schema; unless given, wherever `search_path` resolves its name. */
  readonly migrationTableSchema?: string | null;
  /** The channel a batch that applied a migration notifies, `STATEMENT_CACHE_CHANNEL` unless given; `null` notifies nothing. */
  readonly statementCacheChannel?: string | null;
}

/** A migrator built at run time, pgorm-migration's `Migrations`. */
export declare class Migrations {
  private constructor();
  /**
   * Apply the pending migrations, at most `steps` of them when given, as
   * pgorm-migration's `up` applies them: in one transaction on a connection
   * of `pool`'s own, under the advisory lock, committing after a migration
   * whose `commitsAfter` is true, then clearing the pool's statement caches
   * and notifying the channel. A migration's rejection rolls the batch back,
   * and `up` rejects with it.
   */
  up(pool: Pool, steps?: number | null): Promise<void>;
  /** Create the ledger, and its schema, where either is missing. */
  install(db: Queryable, options?: OperationOptions): Promise<void>;
  /**
   * Every migration with its status, read from the ledger and nothing else.
   * An edited migration whose checksum no longer matches, and a ledger row
   * with no migration, are a `ConstructionError`.
   */
  getMigrationWithStatus(db: Queryable, options?: OperationOptions): Promise<Migration[]>;
  /** The migrations the ledger does not record, in order. */
  getPendingMigrations(db: Queryable, options?: OperationOptions): Promise<Migration[]>;
  /** The migrations the ledger records, in order. */
  getAppliedMigrations(db: Queryable, options?: OperationOptions): Promise<Migration[]>;
  /** Log each migration's name and status with `logger.info`, `console` unless given. */
  status(
    db: Queryable,
    options?: { readonly logger?: { info(message: string): void }; readonly signal?: AbortSignal },
  ): Promise<void>;
}

/**
 * A migrator applying `list` in order, each a `SqlMigration` or a migration
 * written in JavaScript.
 */
export declare function migrations(
  list: readonly (SqlMigration | MigrationTrait)[],
  options?: MigrationsOptions,
): Migrations;
