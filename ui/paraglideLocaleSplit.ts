import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { parseAst, type Plugin } from "vite";

// Paraglide's message-modules output puts every locale's translation of a message
// in that message's module, so the entry chunk carried all locales. In builds this
// plugin keeps only the base locale inline and moves each other locale into its own
// chunk, which `loadLocaleMessages` fetches before the app renders or switches to it.
// Call sites (`m.foo()`) and the runtime are unchanged. In dev nothing is split.

const STORE_ID = "virtual:paraglide-locales";
const LOCALE_PREFIX = "virtual:paraglide-locale/";
const MESSAGE_FILE = /[\\/]src[\\/]paraglide[\\/]messages[\\/](?!_index\.js$)[^\\/]+\.js$/;
const DISPATCH = /^\tif \(locale === "([^"]+)"\) return (\w+)\(inputs\)$/;

type Node = { type: string; start: number; end: number; [key: string]: any };

type ParsedMessage = {
  name: string;
  /** locale → source of that locale's translation function */
  translations: Map<string, string>;
  /** the module with only the base locale and a lookup into loaded locales */
  code: string;
};

function parseMessageModule(code: string, file: string): ParsedMessage {
  const fail = (why: string): never => {
    throw new Error(`paraglide-locale-split: unexpected shape in ${file}: ${why}`);
  };
  const program = parseAst(code) as unknown as { body: Node[] };
  const locals = new Map<string, Node>();
  let exported: Node | undefined;
  let name = "";
  for (const node of program.body) {
    if (node.type === "ImportDeclaration") continue;
    if (node.type === "VariableDeclaration" && node.declarations.length === 1 && node.declarations[0].id.type === "Identifier") {
      locals.set(node.declarations[0].id.name, node);
    } else if (node.type === "ExportNamedDeclaration" && node.declaration?.type === "VariableDeclaration" && !exported) {
      exported = node;
      name = node.declaration.declarations[0].id.name;
    } else {
      fail(`top-level ${node.type}`);
    }
  }
  if (!exported) return fail("no exported message");
  const lines = code.slice(exported.start, exported.end).split("\n");
  const dispatch = lines.flatMap((line, index) => {
    const match = DISPATCH.exec(line);
    return match ? [{ index, locale: match[1], fn: match[2] }] : [];
  });
  const translations = new Map<string, string>();
  const removed: Node[] = [];
  for (const { locale, fn } of dispatch) {
    const decl = locals.get(fn) ?? fail(`missing ${fn}`);
    translations.set(locale, code.slice(decl.declarations[0].init.start, decl.declarations[0].init.end));
    removed.push(decl);
  }
  // Every non-exported declaration must be a dispatched translation or the base one.
  if (locals.size !== removed.length + 1) fail("unexpected local declarations");
  if (dispatch.length) {
    const first = dispatch[0].index;
    if (dispatch.some((entry, i) => entry.index !== first + i)) fail("non-contiguous dispatch");
    if (!/^\treturn \w+\(inputs\)$/.test(lines[first + dispatch.length] ?? "")) fail("no base-locale fallback");
    lines.splice(first, dispatch.length,
      `\tconst __translated = __localeMessages[locale]?.[${JSON.stringify(name)}]`,
      "\tif (__translated) return __translated(inputs)");
  }
  let out = "";
  let cursor = 0;
  for (const decl of removed.sort((a, b) => a.start - b.start)) {
    out += code.slice(cursor, decl.start);
    cursor = decl.end;
  }
  out += code.slice(cursor, exported.start) + lines.join("\n") + code.slice(exported.end);
  if (dispatch.length) out = `import { localeMessages as __localeMessages } from ${JSON.stringify(STORE_ID)};\n${out}`;
  return { name, translations, code: out };
}

export function paraglideLocaleSplit({ messagesDir, locales, baseLocale }: {
  messagesDir: string;
  locales: readonly string[];
  baseLocale: string;
}): Plugin {
  let build = false;
  let parsed: Map<string, ParsedMessage> | null = null;
  const parseFile = (file: string) => parseMessageModule(readFileSync(file, "utf8"), file);
  const allMessages = () => {
    parsed ??= new Map(readdirSync(messagesDir)
      .filter((file) => file.endsWith(".js") && file !== "_index.js")
      .map((file) => [file, parseFile(join(messagesDir, file))]));
    return parsed;
  };
  const otherLocales = locales.filter((locale) => locale !== baseLocale);

  return {
    name: "paraglide-locale-split",
    enforce: "post",
    configResolved(config) { build = config.command === "build"; },
    buildStart() { parsed = null; },
    resolveId(id) {
      if (id === STORE_ID || id.startsWith(LOCALE_PREFIX)) return `\0${id}`;
    },
    load(id) {
      if (id === `\0${STORE_ID}`) {
        if (!build) return "export const localeMessages = {};\nexport async function loadLocaleMessages() {}\n";
        const loaders = otherLocales
          .map((locale) => `  ${JSON.stringify(locale)}: () => import(${JSON.stringify(LOCALE_PREFIX + locale)}),`)
          .join("\n");
        return [
          "export const localeMessages = {};",
          `const loaders = {\n${loaders}\n};`,
          "export async function loadLocaleMessages(locale) {",
          "  if (localeMessages[locale] || !Object.prototype.hasOwnProperty.call(loaders, locale)) return;",
          "  localeMessages[locale] = (await loaders[locale]()).default;",
          "}",
          "",
        ].join("\n");
      }
      if (id.startsWith(`\0${LOCALE_PREFIX}`)) {
        const locale = id.slice(LOCALE_PREFIX.length + 1);
        const entries = [...allMessages().values()].flatMap(({ name, translations }) => {
          const source = translations.get(locale);
          return source ? [`  ${JSON.stringify(name)}: ${source},`] : [];
        });
        return `export default {\n${entries.join("\n")}\n};\n`;
      }
    },
    transform(code, id) {
      if (!build || !MESSAGE_FILE.test(id)) return;
      return { code: parseMessageModule(code, id).code, map: null };
    },
  };
}
