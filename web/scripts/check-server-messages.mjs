// Fails when the server can say something to a person that the Polish interface cannot say.
//
// The server writes English: error messages, notification texts, the status lines of agent
// runs. The interface translates them on arrival (`src/i18n/server.ts`) with the dictionary
// in `src/i18n/server.pl.ts`. This script reads the Rust sources, collects every such text,
// and checks that the dictionary has it:
//
//  - a text without variable parts needs an entry under `exact`;
//  - a `format!` template needs an entry under `patterns` with the same words around the
//    placeholders (`{name}`, `{}` and `{name:?}` all count as "something goes here"), and the
//    translator must really pick that entry for a text made from the template.
//
// What counts as such a text is decided by where it goes, not by a list of texts:
//
//  - ROOTS, FIELD_ROOTS and CALL_ROOTS below name the places a text is handed to: the message
//    of an `ApiError` or a `QueryError`, the text of a notification, the text of an agent
//    run's event, the lines of import reports and of the webhook log, what a module says
//    about itself;
//  - a function that passes one of its own parameters on to such a place becomes such a place
//    itself (`ApiError::bad_request(message)`, `notify_watchers(.., text, ..)`,
//    `tell_requester(.., text)`, `requeue(.., why)`), also when the parameter is only a part
//    of the text (`format!("{what} needs a name")`: every `what` callers pass is checked);
//  - constants, `let` bindings, `if`/`match` arms and helper functions that return
//    `&'static str` are followed to their literals.
//
// EXEMPT names the constructors whose texts no person is meant to read (5xx diagnostics for
// the log). Test code (`tests/` folders, `#[cfg(test)]` items) and demo data (`seed.rs`) are
// not read.
//
// The other way round, every entry of the dictionary must be a text some Rust source really
// has, so that a typo in a key or an entry for a message long gone does not sit there unseen.
//
// Run by `npm run build`; on its own with `npm run check:messages`. `--list` prints every
// text found and where, `--dynamic` the calls whose text is only known at run time, `--extra`
// the dictionary entries for texts that reach the interface some other way.

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { transformWithOxc } from 'vite';

const web = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const root = resolve(web, '..');

/** Where a server text starts its way to a person: the argument `arg` (not counting `self`) of `fn`. */
const ROOTS = [
  { owner: 'ApiError', fn: 'new', arg: 2, what: 'error' },
  { owner: 'QueryError', fn: 'new', arg: 0, what: 'error' },
  { owner: null, fn: 'notify', arg: 4, what: 'notification' },
  { owner: null, fn: 'add_event', arg: 3, what: 'run event' },
];
/** The same for texts that travel in a struct: `NewEvent { text: … }`, `result.outcome = …`. */
const FIELD_ROOTS = [
  { struct: 'NewEvent', field: 'text', what: 'run event' },
  { struct: 'JiraNote', field: 'reason', what: 'import report' },
  { struct: 'JiraTestResult', field: 'message', what: 'connection test' },
  { struct: 'ConfluenceTestResult', field: 'message', what: 'connection test' },
  { struct: 'Ingested', field: 'outcome', assigned: true, what: 'webhook log' },
  { struct: 'ModuleMeta', field: 'name', what: 'module' },
  { struct: 'ModuleMeta', field: 'description', what: 'module' },
];
/** And for texts handed to something that is not a function of ours: a list, an enum variant. */
const CALL_ROOTS = [
  { call: 'unmapped.push', arg: 0, unit: 'modules/jira', what: 'import report' },
  { call: 'problems.push', arg: 0, unit: 'modules/jira', what: 'import report' },
  { call: 'Outcome::Skipped', arg: 0, unit: 'modules/jira', what: 'import report' },
];
/** Constructors whose texts are diagnostics for the log, never shown as they are. */
const EXEMPT = [{ owner: 'ApiError', fn: 'internal' }];
/** Demo data plays the part of people: what it "writes" is not the server speaking. */
const DEMO_DATA = /^seed\.rs$/;

// ---------- reading Rust ----------

function rustFiles(folder) {
  const found = [];

  for (const name of readdirSync(folder).sort()) {
    const path = join(folder, name);

    if (statSync(path).isDirectory()) {
      if (name !== 'tests') {
        found.push(...rustFiles(path));
      }
    } else if (name.endsWith('.rs') && name !== 'tests.rs' && !DEMO_DATA.test(name)) {
      found.push(path);
    }
  }

  return found;
}

function sources() {
  const found = [];

  for (const group of ['crates', 'modules']) {
    for (const name of readdirSync(join(root, group)).sort()) {
      const src = join(root, group, name, 'src');

      if (statSync(join(root, group, name)).isDirectory() && statSync(src, { throwIfNoEntry: false })?.isDirectory()) {
        found.push(...rustFiles(src).map((path) => ({ path, unit: `${group}/${name}` })));
      }
    }
  }

  return found;
}

const ESCAPES = { n: '\n', r: '\r', t: '\t', 0: '\0', '\\': '\\', "'": "'", '"': '"' };

/**
 * The text with comments blanked and the inside of every string and character literal
 * replaced by `_`, so that brackets and names can be found without tripping over prose;
 * and the string literals themselves, by position.
 */
function lex(text) {
  const masked = text.split('');
  const strings = [];
  const fill = (from, to, with_) => {
    for (let at = from; at < to; at += 1) {
      if (masked[at] !== '\n') {
        masked[at] = with_;
      }
    }
  };
  let at = 0;

  while (at < text.length) {
    const char = text[at];

    if (char === '/' && text[at + 1] === '/') {
      const end = text.indexOf('\n', at);
      const to = end === -1 ? text.length : end;

      fill(at, to, ' ');
      at = to;
    } else if (char === '/' && text[at + 1] === '*') {
      let depth = 1;
      let to = at + 2;

      while (to < text.length && depth > 0) {
        if (text.startsWith('/*', to)) {
          depth += 1;
          to += 2;
        } else if (text.startsWith('*/', to)) {
          depth -= 1;
          to += 2;
        } else {
          to += 1;
        }
      }

      fill(at, to, ' ');
      at = to;
    } else if ((char === 'r' || char === 'b') && !/\w/.test(text[at - 1] ?? '') && /^b?r#*"/.test(text.slice(at, at + 20))) {
      const open = /^b?r(#*)"/.exec(text.slice(at, at + 20));
      const start = at + open[0].length;
      const close = text.indexOf(`"${open[1]}`, start);
      const end = close === -1 ? text.length : close;

      strings.push({ start: at, end: end + 1 + open[1].length, value: text.slice(start, end) });
      fill(start, end, '_');
      at = end + 1 + open[1].length;
    } else if (char === '"') {
      let to = at + 1;
      let value = '';

      while (to < text.length && text[to] !== '"') {
        if (text[to] !== '\\') {
          value += text[to];
          to += 1;
        } else if (text[to + 1] === '\n') {
          // A backslash at the end of a line joins it to the next, without the indentation.
          to += 2;

          while (/\s/.test(text[to])) {
            to += 1;
          }
        } else if (text[to + 1] === 'u') {
          const end = text.indexOf('}', to);

          value += String.fromCodePoint(parseInt(text.slice(to + 3, end), 16));
          to = end + 1;
        } else if (text[to + 1] === 'x') {
          value += String.fromCharCode(parseInt(text.slice(to + 2, to + 4), 16));
          to += 4;
        } else {
          value += ESCAPES[text[to + 1]] ?? text[to + 1];
          to += 2;
        }
      }

      strings.push({ start: at, end: to + 1, value });
      fill(at + 1, to, '_');
      at = to + 1;
    } else if (char === "'") {
      // A character ('x', '\n', '\u{1F600}') or a lifetime ('a).
      const literal = /^'(\\u\{[0-9a-fA-F]+\}|\\x[0-9a-fA-F]{2}|\\.|[^\\'])'/u.exec(text.slice(at, at + 14));

      if (literal) {
        fill(at + 1, at + literal[0].length - 1, '_');
        at += literal[0].length;
      } else {
        at += 1;
      }
    } else {
      at += 1;
    }
  }

  return { masked: masked.join(''), strings };
}

const OPEN = { '(': ')', '[': ']', '{': '}' };

/** The index of the bracket that closes the one at `open`. */
function closing(masked, open) {
  let depth = 0;

  for (let at = open; at < masked.length; at += 1) {
    if (masked[at] in OPEN) {
      depth += 1;
    } else if (masked[at] === ')' || masked[at] === ']' || masked[at] === '}') {
      depth -= 1;

      if (depth === 0) {
        return at;
      }
    }
  }

  return masked.length - 1;
}

/** The ranges between top-level commas of `masked[from..to]`. `angles` also nests `<…>`, for parameter lists. */
function split(masked, from, to, angles = false) {
  const parts = [];
  let depth = 0;
  let start = from;

  for (let at = from; at < to; at += 1) {
    const char = masked[at];

    if (char in OPEN || (angles && char === '<')) {
      depth += 1;
    } else if (char === ')' || char === ']' || char === '}' || (angles && char === '>' && masked[at - 1] !== '-')) {
      depth -= 1;
    } else if (char === ',' && depth === 0) {
      parts.push([start, at]);
      start = at + 1;
    }
  }

  if (masked.slice(start, to).trim()) {
    parts.push([start, to]);
  }

  return parts;
}

const STRINGY = /^(&\s*)?('\w+\s+)?(mut\s+)?(str|String)\b|^impl\s+(Into<String>|AsRef<str>|[\w:]*Display)|^Option<&?\s*('\w+\s+)?(str|String)>/;

/** Every `fn` of a file: its name, the type it belongs to, its parameters, where its body is. */
function functions(file) {
  const { masked } = file;
  const impls = [];
  const found = [];

  for (const match of masked.matchAll(/\bimpl\b(?:\s*<[^{;]*?>)?\s+(?:[\w:<>,' ]+?\s+for\s+)?([A-Z]\w*)[^{;]*\{/g)) {
    const open = match.index + match[0].length - 1;

    impls.push({ owner: match[1], start: open, end: closing(masked, open) });
  }

  for (const match of masked.matchAll(/\bfn\s+([A-Za-z_]\w*)/g)) {
    let at = match.index + match[0].length;

    if (masked[at] === '<') {
      for (let depth = 0; at < masked.length; at += 1) {
        if (masked[at] === '<') {
          depth += 1;
        } else if (masked[at] === '>' && masked[at - 1] !== '-') {
          depth -= 1;

          if (depth === 0) {
            at += 1;
            break;
          }
        }
      }
    }

    if (masked[at] !== '(') {
      continue;
    }

    const close = closing(masked, at);
    const params = [];
    let method = false;

    for (const [from, to] of split(masked, at + 1, close, true)) {
      const text = masked.slice(from, to).trim();

      if (/^(&\s*)?('\w+\s+)?(mut\s+)?self\b/.test(text)) {
        method = true;
      } else {
        const named = /^(?:mut\s+)?([a-z_]\w*)\s*:\s*([^]*)$/.exec(text);

        params.push({ name: named?.[1] ?? null, stringy: named ? STRINGY.test(named[2].trim()) : false });
      }
    }

    const brace = masked.slice(close).search(/[{;]/) + close;
    const body = masked[brace] === '{' ? [brace, closing(masked, brace)] : null;
    const head = masked.slice(close + 1, brace);
    const owner = impls.find((impl) => match.index > impl.start && match.index < impl.end)?.owner ?? null;
    const visible = /\bpub\b[^;{}]*$/.test(masked.slice(Math.max(0, match.index - 40), match.index));

    found.push({ name: match[1], owner, method, params, body, at: match.index, visible, file, words: /->\s*&\s*('static\s+)?str\b/.test(head) });
  }

  return found;
}

function load() {
  return sources().map(({ path, unit }) => {
    const text = readFileSync(path, 'utf8');
    const lexed = lex(text);
    let masked = lexed.masked;

    // Test code: the item after `#[cfg(test)]`, to its closing brace or semicolon.
    for (const match of masked.matchAll(/#\[cfg\(test\)\]/g)) {
      const next = masked.slice(match.index + match[0].length).search(/[{;]/) + match.index + match[0].length;
      const end = masked[next] === '{' ? closing(masked, next) : next;

      masked = masked.slice(0, match.index) + masked.slice(match.index, end + 1).replace(/[^\n]/g, ' ') + masked.slice(end + 1);
    }

    const file = { path: relative(root, path), unit, text, masked, strings: lexed.strings.filter((literal) => masked[literal.start] !== ' ') };

    file.functions = functions(file);
    file.constants = new Map();

    for (const match of masked.matchAll(/\bconst\s+([A-Z][A-Z0-9_]*)\s*:\s*&\s*(?:'static\s+)?str\s*=\s*"/g)) {
      file.constants.set(match[1], file.strings.find((literal) => literal.start === match.index + match[0].length - 1));
    }

    return file;
  });
}

// ---------- following a text to its literals ----------

const files = load();
/** Texts found: `{ text, template, exempt, where, kinds, fills }`, by kind and text. */
const messages = new Map();
/** Calls whose text is not written in the source. */
const dynamic = [];
/** Places a text is handed to, by function and argument; those not looked at yet are in `queue`. */
const sinks = new Map();
const queue = [];

const lineOf = (file, at) => file.text.slice(0, at).split('\n').length;
const place = (file, at) => `${file.path}:${lineOf(file, at)}`;
const exemptFn = (fn) => EXEMPT.some((rule) => rule.owner === fn.owner && rule.fn === fn.name);

/**
 * A sink is `{ fn, arg, what, exempt, part }`. `part` is set when what arrives is only a
 * piece of a text (it fills the placeholder `part.hole` of the template `part.of`).
 */
function addSink(sink) {
  const id = `${sink.fn.file.path}:${sink.fn.at}#${sink.arg}`;
  const known = sinks.get(id);

  // Seen as a whole text after having been seen as a piece, or as something a person reads
  // after having been seen as a diagnostic: the stricter reading wins.
  if (!known || (known.part && !sink.part) || (known.exempt && !sink.exempt)) {
    sinks.set(id, sink);
    queue.push(sink);
  }
}

/**
 * A piece of a text is wording when it reads like words ("issue type", "the agent asks: {}"),
 * and data when it does not ("GET", "WEB-9", "/repos/{name}/pulls", "?").
 */
function wording(text) {
  return /[a-z]{2}/.test(text.replace(/\{[^{}]*\}/g, '')) && !/[/_=&]/.test(text);
}

function record(text, template, file, at, sink) {
  if (sink.part && !wording(text)) {
    return null;
  }

  const key = `${template ? 'T' : 'S'}${text}`;
  const known = messages.get(key) ?? { text, template, exempt: true, where: [], kinds: new Set(), fills: new Map() };

  known.exempt &&= sink.exempt;
  known.where.push(place(file, at));
  known.kinds.add(sink.what);
  messages.set(key, known);

  if (sink.part && !template) {
    const fills = sink.part.of.fills.get(sink.part.hole) ?? new Set();

    sink.part.of.fills.set(sink.part.hole, fills.add(text));
  }

  return known;
}

/** The function whose body holds `at`. */
function enclosing(file, at) {
  return file.functions.filter((fn) => fn.body && fn.body[0] < at && at < fn.body[1]).sort((a, b) => b.at - a.at)[0] ?? null;
}

function constant(file, name) {
  return file.constants.get(name) ?? files.find((other) => other.unit === file.unit && other.constants.has(name))?.constants.get(name);
}

/** `&x`, `(x)`, `x.to_string()`, `x.into()` and the like are `x`. */
function bare(masked, from, to) {
  for (;;) {
    const text = masked.slice(from, to);
    const lead = /^\s*(&\s*(mut\s+)?|\*)/.exec(text);
    const tail = /(\.\s*(to_string|into|to_owned|as_str|clone|trim|as_ref|as_deref)\s*\(\s*\)|\?)\s*$/.exec(text);
    const inner = text.trim();

    if (lead) {
      from += lead[0].length;
    } else if (tail) {
      to -= tail[0].length;
    } else if (inner.startsWith('(') && closing(masked, from + text.indexOf('(')) === from + text.indexOf('(') + inner.length - 1) {
      from += text.indexOf('(') + 1;
      to = from + inner.length - 2;
    } else {
      return [from + text.indexOf(inner), from + text.indexOf(inner) + inner.length];
    }
  }
}

/** The placeholders of a `format!` template, in order: `{}` → null, `{name}` → "name", `{0}` → 0. */
function placeholders(template) {
  const found = [];

  for (const match of template.replace(/\{\{|\}\}/g, '  ').matchAll(/\{([^{}:]*)(?::[^{}]*)?\}/g)) {
    found.push(match[1] === '' ? null : /^\d+$/.test(match[1]) ? Number(match[1]) : match[1]);
  }

  return found;
}

/** A template with every placeholder as `{}` and `{{`/`}}` as the braces they stand for. */
function normalise(template) {
  return template.replace(/\{\{|\}\}|\{[^{}]*\}/g, (found) => (found === '{{' ? '\u0001' : found === '}}' ? '\u0002' : '{}'));
}

/**
 * Where the local `name` of `fn` was last given a value before `before`: the range of that
 * expression, and for `let (a, name) = …` which element of the tuple it is.
 */
function local(file, fn, name, before) {
  const body = file.masked.slice(fn.body[0], before);
  const plain = [...body.matchAll(new RegExp(`\\blet\\s+(?:mut\\s+)?${name}\\b[^=;]*=(?!=)`, 'g'))].pop();
  const names = (match) => match[1].split(',').map((part) => part.replace(/^\s*(mut\s+)?/, '').trim());
  const tuple = [...body.matchAll(/\blet\s+\(([^()]*)\)[^=;]*=(?!=)/g)].filter((match) => names(match).includes(name)).pop();
  const last = [plain, tuple].filter(Boolean).sort((a, b) => b.index - a.index)[0];

  if (!last) {
    return null;
  }

  const start = fn.body[0] + last.index + last[0].length;
  let end = start;

  for (let nest = 0; end < before && !(file.masked[end] === ';' && nest === 0); end += 1) {
    nest += file.masked[end] in OPEN ? 1 : ')]}'.includes(file.masked[end]) ? -1 : 0;
  }

  return { range: [start, end], pick: last === tuple ? names(tuple).indexOf(name) : null };
}

/**
 * Whether `name`, read at `at`, was bound again after `from` by something still open there: a
 * closure's parameter (`|error| … error`) or a pattern (`Some(note) => … note`). Such a name
 * is not the `let` or the parameter further up.
 */
function rebound(file, name, from, at) {
  const between = file.masked.slice(from, at);
  const bindings = [
    ...between.matchAll(new RegExp(`\\|[^|\\n]*\\b${name}\\b[^|\\n]*\\|`, 'g')),
    ...between.matchAll(new RegExp(`\\(\\s*(?:ref\\s+|mut\\s+)?${name}\\s*\\)\\s*(?:=>|=(?!=)|if\\b)`, 'g')),
  ];

  return bindings.some((binding) => {
    let nest = 0;

    // Still open: nothing since has closed more brackets than it opened.
    for (const char of between.slice(binding.index + binding[0].length)) {
      nest += char in OPEN ? 1 : ')]}'.includes(char) ? -1 : 0;

      if (nest < 0) {
        return false;
      }
    }

    return true;
  });
}

/** The variable `name`, read at `at`: a local is followed to its value, a parameter makes its function a sink. */
function variable(file, name, at, sink, depth) {
  const fn = enclosing(file, at);
  const given = fn && depth < 5 ? local(file, fn, name, at) : null;

  if (fn && rebound(file, name, given ? given.range[1] : fn.body[0], at)) {
    return false;
  }

  // A `let` of the same name shadows the parameter.
  if (given) {
    return follow(file, given.range[0], given.range[1], sink, depth + 1, given.pick);
  }

  const param = fn?.params.findIndex((candidate) => candidate.name === name) ?? -1;

  if (param !== -1 && fn.params[param].stringy) {
    addSink({ ...sink, fn, arg: param, exempt: sink.exempt || exemptFn(fn) });

    return true;
  }

  return false;
}

/**
 * What the expression `masked[from..to]` of `file` can be: literals and templates are
 * recorded, a parameter of the function around it makes that function a sink. Returns
 * whether anything was learnt.
 */
function follow(file, from, to, sink, depth = 0, pick = null) {
  [from, to] = bare(file.masked, from, to);
  const text = file.masked.slice(from, to);

  if (/^[a-z_]\w*$/.test(text)) {
    return variable(file, text, from, sink, depth);
  }

  return values(file, from, to, sink, depth, pick);
}

/** Wrappers a value is still a value inside of: `Some("…")`, `x.unwrap_or("…")`, `truncate(text, 140)`. */
const TRANSPARENT = /(^|[^\w.!])(Some|Ok|Err|String::from|Cow::Borrowed|Cow::Owned|Box::new|truncate|hub_core::text::truncate)?$|\.\s*(unwrap_or|unwrap_or_else|map_or|map_or_else|then_some|then|or)$/;

/**
 * Literals, constants and `format!` templates in value position anywhere in the expression.
 * With `pick`, the expression gives tuples and only that element of each is the value.
 */
function values(file, from, to, sink, depth, pick = null) {
  const { masked } = file;
  let learnt = false;
  let at = from;

  while (at < to) {
    const rest = masked.slice(at, to);
    const macro = /^format!\s*\(/.exec(rest);
    const word = /^[A-Za-z_][\w:]*/.exec(rest);

    if (macro) {
      const open = at + macro[0].length - 1;
      const close = closing(masked, open);

      learnt = template(file, open, close, sink, depth) || learnt;
      at = close + 1;
    } else if (masked[at] === '"') {
      const literal = file.strings.find((candidate) => candidate.start === at);
      const after = masked.slice(literal.end, to).trimStart();
      const before = masked.slice(from, at).trimEnd();

      // Not a match pattern ("a" => …, "a" | "b"), nor one side of a comparison.
      if (!/^(=>|\|(?!\|)|==|!=)/.test(after) && !/(==|!=|[^|]\|)$/.test(before)) {
        record(literal.value, false, file, at, sink);
        learnt = true;
      }

      at = literal.end;
    } else if (word) {
      const end = at + word[0].length;
      const called = masked.slice(end).match(/^\s*\(/);

      if (/^[A-Z][A-Z0-9_]*$/.test(word[0]) && !called) {
        const literal = constant(file, word[0]);

        if (literal) {
          record(literal.value, false, file, at, sink);
          learnt = true;
        }
      } else if (called && !/\.\s*$/.test(masked.slice(from, at)) && words(file, word[0])) {
        learnt = helper(file, word[0], sink, depth) || learnt;
        at = closing(masked, end + called[0].length - 1) + 1;
        continue;
      }

      at = end;
    } else if (masked[at] === '(' && TRANSPARENT.test(masked.slice(from, at).trimEnd())) {
      const close = closing(masked, at);

      // `Some("a") => …` is a pattern to match, not a value.
      const pattern = /^\s*(=>|\|(?!\|)|if\b)/.test(masked.slice(close + 1, to));
      const tuple = pick !== null && /[^\w.!]$|^$/.test(masked.slice(from, at).trimEnd());
      const parts = split(masked, at + 1, close);

      for (const [start, end] of pattern ? [] : tuple ? parts.slice(pick, pick + 1) : parts) {
        // `|| "…"` and `|x| "…"`: what a closure gives.
        const closure = /^\s*(move\s+)?\|[^|]*\|/.exec(masked.slice(start, end));

        learnt = follow(file, start + (closure?.[0].length ?? 0), end, sink, depth + 1) || learnt;
      }

      at = close + 1;
    } else if (masked[at] === '(' || masked[at] === '[') {
      // The arguments of any other call are not the value.
      at = closing(masked, at) + 1;
    } else {
      at += 1;
    }
  }

  return learnt;
}

/** The helper `name`, when it is a function of this crate that returns `&'static str`. */
function words(file, name) {
  const short = name.split('::').pop();
  const fits = (fn) => fn.name === short && fn.words && fn.body && !fn.method;

  return file.functions.find(fits) ?? files.filter((other) => other.unit === file.unit).flatMap((other) => other.functions).find((fn) => fits(fn) && fn.visible) ?? null;
}

function helper(file, name, sink, depth) {
  const fn = words(file, name);

  return depth > 5 ? true : values(fn.file, fn.body[0] + 1, fn.body[1], sink, depth + 1);
}

/** A `format!(…)`: the template is a text, and whatever fills its placeholders may be wording too. */
function template(file, open, close, sink, depth) {
  const { masked } = file;
  const args = split(masked, open + 1, close);
  const literal = args[0] && file.strings.find((candidate) => candidate.start === bare(masked, ...args[0])[0]);

  if (!literal) {
    return false;
  }

  const holes = placeholders(literal.value);
  const message = record(literal.value, holes.length > 0, file, literal.start, sink);

  if (!message) {
    return true;
  }

  const named = new Map();
  const positional = [];

  for (const [from, to] of args.slice(1)) {
    const assigned = /^\s*([a-z_]\w*)\s*=(?!=)/.exec(masked.slice(from, to));

    if (assigned) {
      named.set(assigned[1], [from + assigned[0].length, to]);
    } else {
      positional.push([from, to]);
    }
  }

  let next = 0;

  holes.forEach((hole, index) => {
    const range = hole === null ? positional[next++] : typeof hole === 'number' ? positional[hole] : named.get(hole);
    const piece = { ...sink, part: { of: message, hole: index } };

    if (range) {
      follow(file, range[0], range[1], piece, depth + 1);
    } else if (typeof hole === 'string') {
      // `{name}` written inline names a variable in scope.
      variable(file, hole, literal.start, piece, depth + 1);
    }
  });

  return true;
}

/** Every call of the sink's function, and what it is given. */
function visit(sink) {
  const { fn } = sink;
  const exempt = sink.exempt || exemptFn(fn);

  for (const file of files) {
    // A private function is called from its own file; a public one from anywhere.
    if (!fn.visible && file !== fn.file) {
      continue;
    }

    for (const match of file.masked.matchAll(new RegExp(`(?<![\\w])${fn.name}\\s*\\(`, 'g'))) {
      const before = file.masked.slice(Math.max(0, match.index - 60), match.index);
      const qualifier = /([A-Za-z_]\w*)\s*::\s*$/.exec(before)?.[1] ?? null;
      const receiver = /([A-Za-z_]\w*)\s*\.\s*$/.exec(before)?.[1] ?? null;

      if (/\bfn\s+$/.test(before)) {
        continue;
      }

      if (fn.owner) {
        // `Type::name(…)`; inside the type also `Self::name(…)` and `self.name(…)`. Another
        // receiver counts when it is named after the type (`importer.note(…)` for `Importer`,
        // `client.get(…)` for `GithubClient`): `value.get(…)` is somebody else's method.
        const inside = file.functions.some((other) => other.owner === fn.owner && other.body && other.body[0] < match.index && match.index < other.body[1]);
        const namesake = receiver !== null && receiver !== 'self' && file.unit === fn.file.unit && fn.owner.toLowerCase().endsWith(receiver.replaceAll('_', '').toLowerCase());

        if (!(qualifier === fn.owner || (inside && (qualifier === 'Self' || receiver === 'self')) || (fn.method && namesake))) {
          continue;
        }
      } else if (/\.\s*$/.test(before) || (qualifier && /^[A-Z]/.test(qualifier))) {
        continue;
      }

      const open = match.index + match[0].length - 1;
      const close = closing(file.masked, open);
      const range = split(file.masked, open + 1, close)[sink.arg];

      if (range && !follow(file, range[0], range[1], { ...sink, exempt }) && !sink.part) {
        dynamic.push({ where: place(file, match.index), what: sink.what, text: file.text.slice(...bare(file.masked, ...range)).replace(/\s+/g, ' '), exempt });
      }
    }
  }
}

for (const rootSink of ROOTS) {
  const fns = files.flatMap((file) => file.functions).filter((fn) => fn.name === rootSink.fn && fn.owner === rootSink.owner && fn.visible);

  if (fns.length === 0) {
    console.error(`check-server-messages: no function ${rootSink.owner ? `${rootSink.owner}::` : ''}${rootSink.fn} in the Rust sources; update ROOTS in ${relative(root, fileURLToPath(import.meta.url))}`);
    process.exit(1);
  }

  for (const fn of fns) {
    addSink({ fn, arg: rootSink.arg, what: rootSink.what, exempt: false, part: null });
  }
}

/** A text given outright at a root that is not a function: followed like any other. */
function given(file, from, to, rule, at) {
  const sink = { fn: { name: rule.struct ?? rule.call, file }, arg: 0, what: rule.what, exempt: false, part: null };

  if (!follow(file, from, to, sink)) {
    dynamic.push({ where: place(file, at), what: rule.what, text: file.text.slice(...bare(file.masked, from, to)).replace(/\s+/g, ' '), exempt: false });
  }
}

for (const rule of FIELD_ROOTS) {
  let seen = 0;

  for (const file of files) {
    const { masked } = file;
    const impls = file.functions.filter((fn) => fn.owner === rule.struct && fn.body);

    // `Struct { field: … }`, and `Self { field: … }` inside the struct's own functions.
    for (const match of masked.matchAll(new RegExp(`\\b(${rule.struct}|Self)\\s*\\{`, 'g'))) {
      const open = match.index + match[0].length - 1;

      if (/\b(struct|impl|for)\s+$/.test(masked.slice(0, match.index)) || (match[1] === 'Self' && !impls.some((fn) => fn.body[0] < open && open < fn.body[1]))) {
        continue;
      }

      for (const [from, to] of split(masked, open + 1, closing(masked, open))) {
        const field = new RegExp(`^\\s*${rule.field}\\s*:(?!:)`).exec(masked.slice(from, to));

        if (field) {
          seen += 1;
          given(file, from + field[0].length, to, rule, from);
        }
      }
    }

    // `x.field = …;`
    for (const match of rule.assigned ? masked.matchAll(new RegExp(`\\.\\s*${rule.field}\\s*=(?!=)`, 'g')) : []) {
      const start = match.index + match[0].length;
      let end = start;

      for (let nest = 0; end < masked.length && !(masked[end] === ';' && nest === 0); end += 1) {
        nest += masked[end] in OPEN ? 1 : ')]}'.includes(masked[end]) ? -1 : 0;
      }

      seen += 1;
      given(file, start, end, rule, match.index);
    }
  }

  if (seen === 0) {
    console.error(`check-server-messages: nothing in the Rust sources sets ${rule.struct}.${rule.field}; update FIELD_ROOTS in ${relative(root, fileURLToPath(import.meta.url))}`);
    process.exit(1);
  }
}

for (const rule of CALL_ROOTS) {
  let seen = 0;

  for (const file of files.filter((candidate) => candidate.unit === rule.unit)) {
    for (const match of file.masked.matchAll(new RegExp(`(?<![\\w])${rule.call.replace('.', '\\s*\\.\\s*')}\\s*\\(`, 'g'))) {
      const open = match.index + match[0].length - 1;
      const range = split(file.masked, open + 1, closing(file.masked, open))[rule.arg];

      if (range) {
        seen += 1;
        given(file, range[0], range[1], rule, match.index);
      }
    }
  }

  if (seen === 0) {
    console.error(`check-server-messages: nothing in ${rule.unit} calls ${rule.call}(…); update CALL_ROOTS in ${relative(root, fileURLToPath(import.meta.url))}`);
    process.exit(1);
  }
}

while (queue.length > 0) {
  visit(queue.shift());
}

// ---------- the dictionary ----------

/** A dependency-free TypeScript file of the interface, loaded here as the module it is. */
async function typescript(path) {
  const { code } = await transformWithOxc(readFileSync(path, 'utf8'), path);

  return import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`);
}

const here = relative(root, fileURLToPath(import.meta.url));
const dictionaryPath = join(web, 'src', 'i18n', 'server.pl.ts');
const { exact, patterns } = await typescript(dictionaryPath);
const { serverTranslator } = await typescript(join(web, 'src', 'i18n', 'server-match.ts'));
const translator = serverTranslator(exact, patterns);
const byShape = new Map(Object.keys(patterns).map((key) => [normalise(key), key]));

/**
 * A text the pattern `key` is meant for: `fill(index, name)` says what goes into each variable
 * part, and by default that is a mark (‹1›, ‹2›, …; QA-1, QA-2, … where an issue key belongs).
 */
function sample(key, fill = () => null) {
  const marks = [];
  let index = 0;
  const text = key.replace(/\{(\w+)\}/g, (_, name) => {
    const given = fill(index, name);

    index += 1;

    if (given !== null) {
      return given;
    }

    marks.push(name === 'key' ? `QA-${index}` : `‹${index}›`);

    return marks[marks.length - 1];
  });

  return { text, marks };
}

const checked = [...messages.values()].filter((message) => !message.exempt).sort((a, b) => a.where[0].localeCompare(b.where[0], 'en', { numeric: true }));
const exempt = [...messages.values()].filter((message) => message.exempt);
const problems = [];
const used = new Set();

for (const message of checked) {
  if (!message.template) {
    if (Object.hasOwn(exact, message.text)) {
      used.add(message.text);
    } else {
      problems.push({ message, why: 'no entry under `exact`' });
    }

    continue;
  }

  const key = byShape.get(normalise(message.text));

  if (key === undefined) {
    problems.push({ message, why: 'no entry under `patterns`' });
    continue;
  }

  used.add(key);

  // A text made from the template must be translated by this entry, with nothing lost.
  const { text, marks } = sample(key);
  const source = translator.source(text);

  if (source !== key) {
    problems.push({ message, why: source === null ? `the entry "${key}" does not match a text made from the template` : `the entry "${source}" takes the text before "${key}" can` });
  } else if (marks.some((mark) => !translator.translate(text).includes(mark))) {
    problems.push({ message, why: `the Polish of "${key}" leaves out a variable part` });
  }

  // Where the server puts one of its own phrases into the template, the phrase must come out in Polish.
  for (const [hole, phrases] of message.fills) {
    const left = [...phrases].filter((phrase) => {
      const filled = sample(key, (index) => (index === hole ? phrase : null)).text;

      return Object.hasOwn(exact, phrase) && exact[phrase] !== phrase && translator.source(filled) === key && translator.translate(filled).includes(phrase);
    });

    if (left.length > 0) {
      problems.push({ message, why: `the Polish of "${key}" leaves in English what the server puts there (${left.slice(0, 3).map((phrase) => `"${phrase}"`).join(', ')}${left.length > 3 ? ', …' : ''}); write that part as {t:…}` });
    }
  }
}

// The other way round: an entry nobody in the Rust sources writes is a typo or a leftover.
// (A whole text may also sit inside a longer literal: the SQL that marks interrupted imports.)
const written = new Set(files.flatMap((file) => file.strings.map((literal) => literal.value)));
const shapes = new Set([...written].map(normalise));
const strays = [
  ...Object.keys(exact).filter((key) => !written.has(key) && ![...written].some((literal) => literal.includes(key))),
  ...Object.keys(patterns).filter((key) => !shapes.has(normalise(key))),
];

// And each pattern on its own: the Polish puts back exactly the parts its key names.
const uneven = Object.entries(patterns).filter(([key, polish]) => {
  const named = [...key.matchAll(/\{(\w+)\}/g)].map((match) => match[1]);
  const put = new Set([...polish.matchAll(/\{(?:t:)?(\w+)\}/g)].map((match) => match[1]));

  return new Set(named).size !== named.length || named.length !== put.size || named.some((name) => !put.has(name));
});

const staticCount = checked.filter((message) => !message.template).length;
const kinds = {};

for (const message of checked) {
  for (const kind of message.kinds) {
    kinds[kind] = (kinds[kind] ?? 0) + 1;
  }
}

if (process.argv.includes('--list')) {
  for (const message of [...checked, ...exempt]) {
    console.log(`${message.exempt ? 'exempt ' : message.template ? 'pattern' : 'exact  '}  ${JSON.stringify(message.text)}  (${[...message.kinds].join(', ')}; ${message.where[0]}${message.where.length > 1 ? ` +${message.where.length - 1}` : ''})`);
  }
}

if (process.argv.includes('--dynamic')) {
  for (const call of dynamic) {
    console.log(`${call.exempt ? 'exempt ' : 'dynamic'}  ${call.where}  ${call.what}: ${call.text}`);
  }
}

if (process.argv.includes('--extra')) {
  for (const key of [...Object.keys(exact), ...Object.keys(patterns)].filter((key) => !used.has(key))) {
    console.log(`extra    ${JSON.stringify(key)}`);
  }
}

if (checked.length < 50) {
  console.error(`check-server-messages: only ${checked.length} texts were found in the Rust sources; the extraction in ${here} no longer fits the code.`);
  process.exit(1);
}

if (problems.length > 0) {
  console.error(`Server texts without Polish (${problems.length}):\n`);

  for (const { message, why } of problems) {
    console.error(`  ${JSON.stringify(message.text)}\n      ${why}; ${[...message.kinds].join(', ')} at ${message.where.slice(0, 3).join(', ')}${message.where.length > 3 ? ', …' : ''}`);
  }

  console.error(`\nAdd them to ${relative(root, dictionaryPath)}: a text without variable parts under \`exact\`, a format! template under \`patterns\`.`);
}

if (strays.length > 0) {
  console.error(`${problems.length > 0 ? '\n' : ''}Entries of ${relative(root, dictionaryPath)} that no Rust source writes (${strays.length}):\n`);
  console.error(strays.map((key) => `  ${JSON.stringify(key)}`).join('\n'));
  console.error('\nThe key must be the server\'s text letter for letter; remove the entry if the server no longer says it.');
}

if (uneven.length > 0) {
  console.error(`${problems.length + strays.length > 0 ? '\n' : ''}Patterns of ${relative(root, dictionaryPath)} whose Polish does not use each variable part of the key, once named (${uneven.length}):\n`);
  console.error(uneven.map(([key, polish]) => `  ${JSON.stringify(key)}\n      ${JSON.stringify(polish)}`).join('\n'));
}

if (problems.length > 0 || strays.length > 0 || uneven.length > 0) {
  process.exit(1);
}

const extra = Object.keys(exact).length + Object.keys(patterns).length - used.size;

console.log(
  `server messages: ok (${checked.length} texts from ${files.length} Rust files have Polish: ${staticCount} exact, ${checked.length - staticCount} patterns; ` +
    `by where they go: ${Object.entries(kinds).map(([kind, count]) => `${kind} ${count}`).join(', ')}; ` +
    `${exempt.length} internal diagnostics exempt; ${extra} more entries are for texts that reach the interface some other way)`,
);
