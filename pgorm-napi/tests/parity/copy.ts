// The COPY family's parity cases, held to `copy.json` as `select.ts`'s are to
// theirs. A COPY binds nothing, so every case's values are empty.
// [spec:pgorm:req:bindings.copy+1/test]

import { type Builder, col, copyFrom, copyTo, deleteFrom, insert, select, Table } from "../../lib/index.js";

const account = new Table("account", { schema: "app" });

export const cases: Record<string, () => Builder> = {
  "from-text-options": () =>
    copyFrom(account, "text")
      .columns(["id", "name"])
      .columns(["note"])
      .delimiter("|")
      .nullString("NULL")
      .defaultString("@default")
      .header()
      .encoding("UTF8")
      .onErrorIgnore({ rejectLimit: 10, logVerbosity: "verbose" })
      .where(col("id").gt(3))
      .where(col("name").ne("O'Brien")),
  "from-csv-forced": () =>
    copyFrom("ticket", "csv")
      .columns(["id", "note", "label"])
      .quote("'")
      .escape("\\")
      .headerMatch()
      .forceNotNull(["note"])
      .forceNullAll()
      .onErrorIgnore({ logVerbosity: "silent" }),
  "from-binary-freeze": () => copyFrom(new Table("ticket"), "binary").columns(["id"]).freeze(),
  "to-table-text": () => copyTo(account, "text").nullString("").encoding("LATIN1"),
  "to-select-csv": () =>
    copyTo(select(col("id"), col("name")).from(account).where(col("name").eq("O'Brien")), "csv")
      .header()
      .delimiter(";")
      .nullString("-")
      .quote('"')
      .escape('"')
      .forceQuote(["name"]),
  "to-delete-returning": () =>
    copyTo(deleteFrom(account).where(col("id").gt(3)).returning([]), "csv").forceQuoteAll(),
  "to-insert-returning": () => copyTo(insert(new Table("ticket")).columns("note").values("x").returning([col("id")]), "binary"),
};
