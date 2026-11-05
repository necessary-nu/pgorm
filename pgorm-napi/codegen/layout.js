// @ts-check

// TypeScript and JavaScript laid out as a person writes them, in the layout
// deno fmt and prettier give: 80 columns, a bracket's items on its line where
// they fit and otherwise one to a line with a comma after each, and a doc
// comment wrapped to the width.
// [spec:pgorm:req:bindings.codegen-layout]

export const WIDTH = 80;

/**
 * `head`, which opens a bracket, then `items`, `close` and `trailer`, at
 * `indent`: on one line where it fits, else one item to a line, each with a
 * comma after it unless `comma` is false, as a type argument list takes
 * none, and the lines of an item that is several indented with it.
 *
 * @param {string} head
 * @param {readonly string[]} items
 * @param {string} close
 * @param {string} [trailer]
 * @param {number} [indent]
 * @param {boolean} [comma]
 */
export function bracket(head, items, close, trailer = "", indent = 0, comma = true) {
  const pad = " ".repeat(indent);
  const line = `${pad}${head}${items.join(", ")}${close}${trailer}`;
  if (line.length <= WIDTH && !items.some((item) => item.includes("\n"))) return line;
  const inner = " ".repeat(indent + 2);
  const body = items.map((item, index) =>
    `${inner}${item.replaceAll("\n", `\n${inner}`)}${comma || index < items.length - 1 ? "," : ""}`
  );
  return [`${pad}${head}`, ...body, `${pad}${close}${trailer}`].join("\n");
}

/**
 * A JSON value as a JavaScript literal, keys that are identifiers unquoted:
 * on one line where `before` and `after` columns around it leave it room at
 * `indent`, else an entry to a line, each followed by a comma and laid out
 * the same way in turn, its lines after the first indented from where it
 * starts, as `bracket` takes an item.
 *
 * @param {unknown} value
 * @param {number} [indent]
 * @param {number} [before]
 * @param {number} [after]
 * @returns {string}
 */
export function literal(value, indent = 0, before = 0, after = 0) {
  const line = flat(value);
  const container = Array.isArray(value) ? value.length > 0 : value !== null && typeof value === "object" && Object.keys(value).length > 0;
  if (!container || indent + before + line.length + after <= WIDTH) return line;
  const entries = Array.isArray(value)
    ? value.map((item) => literal(item, indent + 2, 0, 1))
    : Object.entries(/** @type {Record<string, unknown>} */ (value)).map(([key, item]) => {
      const label = `${property(key)}: `;
      return `${label}${literal(item, indent + 2, label.length, 1)}`;
    });
  const [open, close] = Array.isArray(value) ? ["[", "]"] : ["{", "}"];
  return [open, ...entries.map((entry) => `  ${entry.replaceAll("\n", "\n  ")},`), close].join("\n");
}

/** @param {unknown} value @returns {string} */
function flat(value) {
  if (Array.isArray(value)) return `[${value.map(flat).join(", ")}]`;
  if (value !== null && typeof value === "object") {
    const entries = Object.entries(value).map(([key, item]) => `${property(key)}: ${flat(item)}`);
    return entries.length === 0 ? "{}" : `{ ${entries.join(", ")} }`;
  }
  return JSON.stringify(value);
}

/** @param {string} key */
function property(key) {
  return /^[A-Za-z_$][A-Za-z0-9_$]*$/.test(key) ? key : JSON.stringify(key);
}

/**
 * An object type of `members`, each `name: type`, on one line where
 * `before` and `after` columns around it leave it room at `indent`, else a
 * member to a line, each ending `;`.
 *
 * @param {readonly string[]} members
 * @param {number} indent
 * @param {number} before
 * @param {number} after
 */
export function objectType(members, indent, before, after) {
  const line = `{ ${members.join("; ")} }`;
  if (indent + before + line.length + after <= WIDTH) return line;
  const inner = " ".repeat(indent + 2);
  return ["{", ...members.map((member) => `${inner}${member};`), `${" ".repeat(indent)}}`].join("\n");
}

/**
 * A union of `members` after `head`, ending `;`: on one line where it fits,
 * else a member to a line, each led by its `|`.
 *
 * @param {string} head
 * @param {readonly string[]} members
 */
export function union(head, members) {
  const line = `${head} ${members.join(" | ")};`;
  if (line.length <= WIDTH) return line;
  return [head, ...members.map((member, index) => `  | ${member}${index === members.length - 1 ? ";" : ""}`)].join("\n");
}

/**
 * A `/** .. *\/` comment of `text` at `indent`, on one line where it fits
 * and otherwise wrapped at word boundaries to the width.
 *
 * @param {string} text
 * @param {number} [indent]
 */
export function docComment(text, indent = 0) {
  const pad = " ".repeat(indent);
  const line = `${pad}/** ${text} */`;
  if (line.length <= WIDTH) return line;
  const lines = [];
  let current = "";
  for (const word of text.split(" ")) {
    if (current !== "" && pad.length + 3 + current.length + 1 + word.length > WIDTH) {
      lines.push(current);
      current = word;
    } else {
      current = current === "" ? word : `${current} ${word}`;
    }
  }
  lines.push(current);
  return [`${pad}/**`, ...lines.map((part) => `${pad} * ${part}`), `${pad} */`].join("\n");
}

/**
 * `import { names } from "source";`, `type` imports where `kind` says so,
 * on one line where it fits and otherwise a name to a line.
 *
 * @param {readonly string[]} names
 * @param {string} source
 * @param {string} [kind]
 */
export function importOf(names, source, kind = "") {
  const from = `} from ${JSON.stringify(source)};`;
  const line = `import ${kind}{ ${names.join(", ")} ${from}`;
  if (line.length <= WIDTH) return line;
  return [`import ${kind}{`, ...names.map((name) => `  ${name},`), from].join("\n");
}
