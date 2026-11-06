// @ts-check

// A registered entity's schema, written by pgorm's own entity schema path —
// `Schema::create_table_from_entity` with its enum, index and comment
// statements — as the module's DDL statements, which the terminals run as
// they run any other. A table that path refuses, such as one whose foreign
// key PostgreSQL cannot build, is a ConstructionError carrying the Rust
// message, never a schema with the key left out.
// [spec:pgorm:req:bindings.schema-from-entity]

import { arg, TRUSTED } from "./builder.js";
import { Entity } from "./entities.js";
import { ConstructionError } from "./errors.js";
import { described, sameTable, shapeOf } from "./model-shape.js";
import { Model } from "./models.js";
import { foreignKeyOf, Relation, shapesOf } from "./relations.js";
import { native } from "./operations.js";
import { CreateTable, DDL, statement } from "./schema.js";
import { CreateIndex } from "./schema-objects.js";
import { CreateType } from "./schema-types.js";

/** Marks a construction from within the module. */
const MADE = Symbol("pgorm-napi entity schema");

/**
 * A registered entity's schema: its table, the enum types its columns name,
 * its indexes and its comments, each in the order pgorm writes them.
 */
export class EntitySchema {
  /** @readonly @type {CreateTable} */
  table;
  /** @readonly @type {readonly CreateType[]} */
  enums;
  /** @readonly @type {readonly CreateIndex[]} */
  indexes;
  /** @readonly @type {readonly DDL[]} */
  comments;

  /**
   * @param {[unknown, unknown[], unknown[], unknown[]]} parts
   * @param {symbol} token
   */
  constructor([table, enums, indexes, comments], token) {
    if (token !== MADE) throw new TypeError("an EntitySchema comes from fromEntity(entity) or fromModel(model)");
    this.table = new CreateTable(table, TRUSTED);
    this.enums = Object.freeze(enums.map((handle) => new CreateType(handle, TRUSTED)));
    this.indexes = Object.freeze(indexes.map((handle) => new CreateIndex(handle, TRUSTED)));
    this.comments = Object.freeze(comments.map(statement));
    Object.freeze(this);
  }
}

/**
 * The schema of a registered entity, by pgorm's own entity schema path.
 *
 * @param {Entity} entity
 */
export function fromEntity(entity) {
  if (!(entity instanceof Entity)) {
    throw new TypeError("fromEntity takes a registered Entity, from entity(name); a declared model's is fromModel(model)");
  }
  return new EntitySchema(native.entitySchema(arg(entity)), MADE);
}

// [spec:pgorm:req:bindings.schema-from-model]
/**
 * The schema of a declared model's table, written by pgorm's `Schema` from
 * the `TableShape` the model describes, as `fromEntity` writes a registered
 * entity's: its `CREATE TABLE` with its primary key, unique keys and a
 * foreign key for each `belongsTo` relation in `relations`, the `CREATE
 * TYPE` of each enum its columns use, an index for each `indexed` column,
 * and its comments.
 *
 * @param {unknown} model
 * @param {{ relations?: unknown }} [options]
 */
export function fromModel(model, options = {}) {
  if (!(model instanceof Model)) {
    throw new TypeError("fromModel takes a declared Model, from model(name, ..); a registered entity's is fromEntity(entity)");
  }
  if (typeof options !== "object" || options === null) throw new TypeError("fromModel's options are { relations }");
  for (const option of Object.keys(options)) {
    if (option !== "relations") throw new TypeError(`${JSON.stringify(option)} is no fromModel option`);
  }
  const shape = shapeOf(model);
  if (shape.alias !== null) {
    throw new ConstructionError("a model's table is created under its name: pass the model, not an alias of it");
  }
  const { relations = [] } = options;
  if (!Array.isArray(relations)) throw new TypeError("fromModel's relations are an array of relations");
  const keys = relations.map((relation) => {
    if (!(relation instanceof Relation)) throw new TypeError("fromModel's relations are the model's belongsTo relations");
    if (relation.kind !== "belongsTo") {
      throw new ConstructionError(
        `a ${relation.kind} relation holds no foreign key of its model's table: pass the belongsTo relation of the table that holds it`,
      );
    }
    const [from] = shapesOf(relation);
    if (!sameTable(from, shape)) {
      throw new ConstructionError(`the relation starts at ${from.name}, not at ${shape.name}`);
    }
    return foreignKeyOf(relation);
  });
  return new EntitySchema(native.modelSchema(described(shape), keys), MADE);
}
