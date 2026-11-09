// [spec:pgorm:req:bindings.models-from-sql]

/** A table, by its name or by its schema and name. */
export type CodegenTable = string | readonly [schema: string, name: string];

/**
 * A base table and its kinds' tables, read as an unkinded disjoint model, as
 * pgorm-codegen's `UnkindedDisjoint` names one: the base, each kind's table in
 * the order its kinds take, and the value of the kind a base row with a row
 * in no kind's table is, if such a row is one.
 */
export interface UnkindedDisjointTables {
  readonly base: CodegenTable;
  readonly kinds: readonly CodegenTable[];
  readonly unit?: string | null;
}

/** What {@link modelsFromSql} generates beside the DDL. */
export interface ModelsFromSqlOptions {
  /** The schema an unqualified table and type are in; a type is in `public` by default. */
  readonly schemaName?: string | null;
  readonly unkindedDisjoint?: readonly UnkindedDisjointTables[];
}

/**
 * The TypeScript module declaring the models of the tables `ddl` creates, as
 * pgorm-codegen reads them for Rust entities: each table a `model(..)`, each
 * foreign key a `belongsTo` relation and the other side's `hasOne` or
 * `hasMany`, each junction a `related` path, and the tables of a disjoint
 * enum and a one-of reference's `CHECK` their `disjoint` and `oneOf`. DDL
 * pgorm-codegen refuses, and anything a declared model cannot say, is a
 * {@link ConstructionError} naming it.
 */
export declare function modelsFromSql(ddl: string, options?: ModelsFromSqlOptions): string;
