// A consumer of the generated module, held by `deno check`: each assignment
// type-checks only if the declarations type the application's records,
// writes, graph rows and source rows from its registrations, and each
// `@ts-expect-error` only if they refuse what the module refuses.
// [spec:pgorm:req:napi.codegen-types/test]

import type { Decimal, Pool, Uuid } from "../lib/index.js";
import { pipeline as pl } from "../lib/index.js";
import {
  Account,
  type AccountNotesRow,
  AccountNotes,
  type AccountRecord,
  AccountWithNote,
  Deliveries,
  type DeliveriesRow,
  Delivery,
  MixedNotes,
  Note,
  Offers,
  type OffersRow,
  Parcel,
  type ParcelRecord,
  Sample,
  type SampleRecord,
} from "../lib/app.js";

/** Never called: its body is what `deno check` holds the declarations to. */
export async function typed(pool: Pool): Promise<unknown[]> {
  const account: AccountRecord = await Account.find().where(Account.col("mood").eq("busy")).one(pool);
  const id: number = account.id;
  const name: string = account["display name"];
  const note: string | null = account.note;
  const mood: "calm" | "busy" = account.mood;
  // @ts-expect-error: no such column
  account.nickname;
  // @ts-expect-error: a mood is one of the enum's labels
  Account.col("mood").eq("wild");
  // @ts-expect-error: an id is a number
  Account.active().set("id", "one");
  // @ts-expect-error: no such column
  Account.active().set("nickname", "x");
  const sample: SampleRecord = await Sample.find().one(pool);
  const big: bigint = sample.id;
  const price: Decimal = sample.price;
  const token: Uuid = sample.token;
  const day: Temporal.PlainDate = sample.day;
  const at: Temporal.Instant = sample.at;
  const tags: string[] = sample.tags;
  // @ts-expect-error: a Vec<String> holds no NULL
  Sample.active().set("tags", ["x", null]);
  const written = Sample.active().set("id", 1).set("id", 1n).set("note", null);
  const rows: AccountNotesRow[] = await AccountNotes.find({ aliases: ["n"] }).all(pool);
  const [root, attached] = rows[0]!;
  const body: string | undefined = attached?.body;
  // @ts-expect-error: an optional slot may be null
  const required: string = rows[0]![1].body;
  const [, kept, maybe] = (await MixedNotes.find().all(pool))[0]!;
  const keptBody: string = kept.body;
  const maybeBody: string | undefined = maybe?.body;
  const selected = await pl.from(Account).selectSources(AccountWithNote).all(pool);
  const [first, second] = selected[0]!;
  const selectedName: string | undefined = first?.["display name"];
  const selectedBody: string | undefined = second?.body;
  const noteRecord = await Note.find().optional(pool);
  // [spec:pgorm:req:bindings.entity-inserts/test]    an entity's insert types its records and keys by row count, a batch made by add
  const insertedAccount: AccountRecord = await Account.insert(Account.active().set("id", 1)).returningModel(pool);
  const insertedAccounts: AccountRecord[] = await Account.insert(Account.active()).add(Account.active()).returningModels(pool);
  const insertedKeys = await Account.insertMany([Account.active()]).addMany([Account.active()]).returningPks(pool);
  // @ts-expect-error: a batch answers with every row's key
  Account.insertMany([Account.active()]).returningPk(pool);
  // @ts-expect-error: an insert of one row answers with its own
  Account.insert(Account.active()).returningModels(pool);
  // [spec:pgorm:req:bindings.codegen-disjoint/test]    a disjoint enum's row is a union over its kinds that narrows by kind, its writes take its entities' ActiveModels and its kinds, and an unkinded enum's has no write
  const delivered: DeliveriesRow = await Deliveries.insert(Delivery.active().set("label", "box"), "parcel", Parcel.active().set("weight_g", 5))
    .returningModel(pool);
  const [delivery, deliveredKind, deliveredRow] = delivered;
  const label: string = delivery.label;
  if (deliveredKind === "parcel") {
    const parcel: ParcelRecord = deliveredRow;
    void parcel;
  }
  if (delivered[1] === "letter") {
    const none: null = delivered[2];
    void none;
  }
  // @ts-expect-error: boat is no kind of the enum
  Deliveries.insert(Delivery.active(), "boat");
  // @ts-expect-error: a note's ActiveModel is no delivery's
  Deliveries.insert(Note.active(), "letter");
  const listed: OffersRow[] = await Offers.find().all(pool);
  // @ts-expect-error: an unkinded enum is read-only
  Offers.insert(Delivery.active(), "draft");
  return [id, name, note, mood, big, price, token, day, at, tags, written, root, body, required, keptBody, maybeBody,
    selectedName, selectedBody, noteRecord, insertedAccount, insertedAccounts, insertedKeys, label, listed];
}
