/**
 * One-of references: a model's nullable fields, each holding the key of one
 * target model, of which a row references exactly one, or at most one where
 * the reference is optional.
 *
 * @module
 */

// [spec:pgorm:req:napi.typing+1]
// [spec:pgorm:req:bindings.one-of+1]

import type { ActiveModel, EntityRecord } from "./entities.d.ts";
import type { Queryable } from "./index.d.ts";
import type { ColumnsOf, FieldOf, Model, RowOf, UpdateOf } from "./models.d.ts";
import type { AlterTable } from "./schema.d.ts";

/** Each target's value, in order, to the field holding its key and the target's model. */
export type OneOfTargets<M extends Model<any>> = {
  readonly [value: string]: readonly [FieldOf<ColumnsOf<M>>, Model<any>];
};

/** A record's target record, tagged by its target's value. */
export type OneOfLoaded<T> = {
  [V in keyof T & string]: T[V] extends readonly [string, infer N] ? [V, RowOf<N>] : never;
}[keyof T & string];

/** A one-of reference's declaration as data. */
export interface OneOfDescription {
  readonly name: string;
  readonly model: string;
  readonly targets: readonly { readonly value: string; readonly field: string; readonly target: string }[];
  readonly optional: boolean;
}

/**
 * A reference from a model's rows to exactly one of its targets, or to at
 * most one where it is optional, made by {@link oneOf}.
 */
export declare class OneOf<M extends Model<any> = Model<any>, T extends OneOfTargets<M> = OneOfTargets<M>> {
  private constructor();
  readonly name: string;
  describe(): OneOfDescription;
  /**
   * The record's reference as `[value, key]`, or `null` for a row
   * referencing none where the reference is optional; a row against the
   * declaration is a {@link DecodeError}.
   */
  key(record: RowOf<M>): [keyof T & string, unknown] | null;
  /**
   * The fields a write sets: the target's field set to `key` and every
   * other to `null`, each `null` for `set(null)` where the reference is
   * optional.
   */
  set(value: (keyof T & string) | null, key?: unknown): UpdateOf<M>;
  /** Each record's target record, tagged, in order, or `null` for a record referencing none. */
  load(db: Queryable, records: readonly RowOf<M>[]): Promise<(OneOfLoaded<T> | null)[]>;
}

/**
 * A downstream crate's registered `#[derive(OneOf)]` enum, made by
 * {@link oneOfEntity}: its targets named as its variants are, in snake_case,
 * and its records the entities' own.
 */
export declare class OneOfEntity {
  private constructor();
  readonly name: string;
  describe(): OneOfDescription;
  /** The record's reference as `[value, key]`, or `null` for none where the reference is optional. */
  key(record: EntityRecord): [string, unknown] | null;
  /** `active` with its reference set: the target's column to `key`, every other to null. */
  set(active: ActiveModel, value: string | null, key?: unknown): ActiveModel;
  /** Each record's target record, tagged, in order, or `null` for a record referencing none. */
  load(db: Queryable, records: readonly EntityRecord[]): Promise<([string, EntityRecord] | null)[]>;
}

/**
 * Declare a reference from `model`'s rows to exactly one of `targets`, or to
 * at most one where `optional` says so. What the derive refuses is a
 * {@link ConstructionError} thrown as the reference is declared.
 */
export declare function oneOf<M extends Model<any>, const T extends OneOfTargets<M>>(
  name: string,
  model: M,
  targets: T,
  options?: { readonly optional?: boolean },
): OneOf<M, T>;

/** The `#[derive(OneOf)]` enum the module registers as `name`. */
export declare function oneOfEntity(name: string): OneOfEntity;

/** The names of the one-of enums the module registers, in order. */
export declare function oneOfEntities(): string[];

/** The `ALTER TABLE` adding the reference's `CHECK`. */
export declare function createOneOf(reference: OneOf<any, any> | OneOfEntity): AlterTable;
