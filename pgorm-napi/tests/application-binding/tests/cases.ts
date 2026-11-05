// The entity family's parity cases, built through the binding from the
// application's registrations and held to `parity.json`, which the
// application crate's Rust test builds with its entities directly.
// [spec:pgorm:req:bindings.entity-reads+3/test]

import { type Compiled, entity, graph, pipeline as pl, select, Table } from "../../../lib/index.js";

export function cases(): Record<string, () => Compiled> {
  const Account = entity("app.Account");
  const Membership = entity("app.Membership");
  const Parcel = entity("app.Parcel");
  const notes = graph("app.AccountNotes").find({ aliases: ["n"] });
  const joined = () =>
    pl.from(Account).join(pl.source(entity("app.Note")).named("n"), pl.col("accounts", "id").eq(pl.col("n", "account_id")), {
      kind: "left",
    });
  return {
    "find": () => Account.find().inspect(),
    "find-filtered": () =>
      Account.find().where(Account.col("id").gte(10)).orderBy(Account.col("display name").desc()).limit(20).inspect(),
    "find-enum": () => Account.find().where(Account.col("mood").eq("busy")).inspect(),
    "find-one": () => Account.find().inspect("one"),
    "find-one-limit-zero": () => Account.find().limit(0).inspect("one"),
    "find-composite": () =>
      Membership.find().where(Membership.col("account_id").eq(1)).where(Membership.col("team").eq("red")).inspect(),
    "graph-notes": () => notes.inspect(),
    "graph-mixed-default-aliases": () => graph("app.MixedNotes").find().inspect(),
    "graph-filtered": () => notes.where(notes.col(1, "body").eq("x")).orderBy(notes.col(0, "id").asc()).inspect(),
    "find-member-subquery": () =>
      Parcel.find().where(
        Parcel.col("mood").isIn(
          select(Parcel.col("mood")).from(new Table("parcels", { schema: "napi_entities" })).where(Parcel.col("id").eq(2)),
        ),
      ).inspect(),
    "find-member-values": () => Parcel.find().where(Parcel.col("mood").isNotIn(["busy"])).inspect(),
    "pipeline-registered-column": () =>
      pl.from(Parcel).filter(pl.col("parcels", Parcel.col("mood")).eq("busy")).inspect(),
    "pipeline-entity": () => pl.from(Account).inspect(),
    "sources-single": () => pl.from(Account).selectSources(pl.sources("app.SingleAccount")).inspect(),
    "sources-joined": () =>
      joined().selectSources(pl.sources("app.AccountWithNote"), { qualifiers: ["accounts", "n"] }).inspect(),
    "sources-joined-one": () =>
      joined().selectSources(pl.sources("app.AccountWithNote"), { qualifiers: ["accounts", "n"] }).inspect("optional"),
  };
}
