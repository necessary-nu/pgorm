// The values index.d.ts declares are the ones the module exports, held by
// `deno check`: nothing here runs, and neither runner collects the file. The
// suite and an application type-check their imports against the
// declarations, so a declared value the module does not make would pass
// `deno check` and fail only when the import links; an export left
// undeclared breaks `napi.typing`. A failure names the value in a
// `Record<"name", never>` the empty object lacks.
// [spec:pgorm:req:napi.linking/test]
// [spec:pgorm:req:napi.typing+1/test]

import type * as declared from "../lib/index.d.ts";
import type * as delivered from "../lib/pgorm.js";

/** Each name of `From`'s that `In` lacks, as a property the empty object cannot have. */
type Lacking<From, In> = Record<Exclude<keyof From, keyof In>, never>;

export const undelivered: Lacking<typeof declared, typeof delivered> = {};
export const undeclared: Lacking<typeof delivered, typeof declared> = {};
export const undeliveredPipeline: Lacking<typeof declared.pipeline, typeof delivered.pipeline> = {};
export const undeclaredPipeline: Lacking<typeof delivered.pipeline, typeof declared.pipeline> = {};
