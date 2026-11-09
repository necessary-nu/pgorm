// @ts-check
// @ts-self-types="./index.d.ts"

// What an application imports, typed by index.d.ts. Deno takes a module's
// types from its @ts-self-types file and resolves none of the module's own
// imports, so an import here that names a missing export would pass
// `deno check` and fail only when the module links. The module is therefore
// pgorm.js, whose imports `deno check` holds, and tests/declarations.ts holds
// index.d.ts's values to the ones pgorm.js exports.
// [spec:pgorm:req:napi.typing+1]
// [spec:pgorm:req:napi.linking]

export * from "./pgorm.js";
