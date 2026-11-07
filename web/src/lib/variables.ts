/**
 * `{{first_name}}` and the like in a message body. The server fills them in as a message is
 * queued (`server/src/scheduler/variables.rs`); this side offers them while typing, checks them
 * the same way, and previews the result.
 */

/** What's known about a message while it's written, for examples and the preview. */
export interface VariableContext {
  /** Whether a recipient is chosen yet; until then names are unknown rather than missing. */
  hasRecipient: boolean
  firstName: string | null
  name: string | null
  /** When it goes out; `null` until a time is set. */
  at: Date | null
  timeZone: string
}

interface Variable {
  name: string
  /** What the variable holds, as a phrase that reads after its example. */
  hint: string
  value: (context: VariableContext) => string | null
}

const inZone = (options: Intl.DateTimeFormatOptions, locale = 'en-GB') => {
  return (context: VariableContext) =>
    new Intl.DateTimeFormat(locale, {
      ...options,
      timeZone: context.timeZone,
    }).format(context.at ?? new Date())
}

// Formats match the server's, which fills these in English
export const VARIABLES: Variable[] = [
  {
    name: 'first_name',
    hint: 'their first name, as saved on your phone',
    value: (context) => context.firstName,
  },
  {
    name: 'name',
    hint: 'their name as saved on your phone, or the group’s name',
    value: (context) => context.name,
  },
  {
    name: 'day',
    hint: 'the day it goes out',
    value: inZone({ weekday: 'long' }),
  },
  {
    name: 'date',
    hint: 'the date it goes out',
    value: inZone({ day: 'numeric', month: 'long' }),
  },
  {
    name: 'time',
    hint: 'the time it goes out',
    value: inZone(
      { hour: 'numeric', minute: '2-digit', hour12: true },
      'en-US',
    ),
  },
  {
    name: 'month',
    hint: 'the month it goes out',
    value: inZone({ month: 'long' }),
  },
  {
    name: 'year',
    hint: 'the year it goes out',
    value: inZone({ year: 'numeric' }),
  },
]

const NAMES = new Set(VARIABLES.map((variable) => variable.name))

/** A variable being typed, or the one the cursor sits in. */
export interface OpenVariable {
  /** Where its `{{` starts. */
  start: number
  /** What's typed of its name so far. */
  query: string
  /** The whole name when the cursor sits inside one, `{{da|te}}` giving `date`. */
  current: string
}

/** The variable the cursor is in, from `{{` up to the cursor, while it's still a name. */
export function variableAt(text: string, caret: number): OpenVariable | null {
  const start = text.lastIndexOf('{{', caret - 2)
  if (start === -1) return null
  const typed = text.slice(start + 2, caret)
  if (!/^\s*[a-z_]*$/i.test(typed)) return null
  const query = typed.trim().toLowerCase()
  const rest = /^[a-z_]*/i.exec(text.slice(caret))?.[0] ?? ''
  return { start, query, current: (query + rest).toLowerCase() }
}

/** Names that hold the query, the ones it starts first. */
export function matchingVariables(query: string) {
  const found = VARIABLES.filter((variable) => variable.name.includes(query))
  return [
    ...found.filter((variable) => variable.name.startsWith(query)),
    ...found.filter((variable) => !variable.name.startsWith(query)),
  ]
}

/**
 * Puts `name` in place of what's typed. A `}}` already there, with any fallback before it, stays,
 * so picking inside `{{nme|there}}` keeps the fallback. Returns the text and where the cursor goes.
 */
export function insertVariable(
  text: string,
  caret: number,
  open: OpenVariable,
  name: string,
) {
  const rest = text.slice(caret)
  const nameEnd = caret + (/^[a-z_]*/i.exec(rest)?.[0].length ?? 0)
  const after = text.slice(nameEnd)
  const close = /^\s*(\|[^}\n]*)?\}\}/.exec(after)
  const before = `${text.slice(0, open.start)}{{${name}`
  return close
    ? { text: before + after, caret: before.length + close[0].length }
    : { text: `${before}}}${after}`, caret: before.length + 2 }
}

type Piece =
  | { kind: 'text'; text: string }
  | { kind: 'variable'; name: string; fallback: string | null }

/** The body in parts, or what's wrong with it, worded as the server words it. */
function parse(body: string): Piece[] | string {
  const pieces: Piece[] = []
  let rest = body
  for (;;) {
    const open = rest.indexOf('{{')
    if (open === -1) break
    const close = rest.indexOf('}}', open + 2)
    if (close === -1) return 'A {{ needs a }} to close it'
    const inner = rest.slice(open + 2, close)
    const bar = inner.indexOf('|')
    const name = (bar === -1 ? inner : inner.slice(0, bar)).trim()
    if (!NAMES.has(name)) {
      return name
        ? `{{${name}}} isn’t a variable`
        : '{{}} needs a variable’s name inside'
    }
    if (open > 0) pieces.push({ kind: 'text', text: rest.slice(0, open) })
    pieces.push({
      kind: 'variable',
      name,
      fallback: bar === -1 ? null : inner.slice(bar + 1).trim(),
    })
    rest = rest.slice(close + 2)
  }
  if (rest) pieces.push({ kind: 'text', text: rest })
  return pieces
}

/** Whether a whole `{{…}}` names a variable, with or without a fallback. */
export function isVariable(token: string) {
  const inner = token.slice(2, -2)
  const bar = inner.indexOf('|')
  return NAMES.has((bar === -1 ? inner : inner.slice(0, bar)).trim())
}

/** What's wrong with a body's variables, if anything. */
export function variableProblem(body: string) {
  const parsed = parse(body)
  return typeof parsed === 'string' ? parsed : null
}

export function hasVariables(body: string) {
  const parsed = parse(body)
  return (
    typeof parsed !== 'string' &&
    parsed.some((piece) => piece.kind === 'variable')
  )
}

/** The body as it would go out with what's known now. */
export function preview(body: string, context: VariableContext) {
  const parsed = parse(body)
  if (typeof parsed === 'string') return body
  return parsed
    .map((piece) => {
      if (piece.kind === 'text') return piece.text
      const value = VARIABLES.find(
        (variable) => variable.name === piece.name,
      )?.value(context)
      return value || piece.fallback || ''
    })
    .join('')
}

/** Variables in the body that would go out empty, having neither a value nor a fallback. */
export function emptyVariables(body: string, context: VariableContext) {
  const parsed = parse(body)
  if (typeof parsed === 'string' || !context.hasRecipient) return []
  const names = parsed.flatMap((piece) =>
    piece.kind === 'variable' &&
    piece.fallback === null &&
    !VARIABLES.find((variable) => variable.name === piece.name)?.value(context)
      ? [piece.name]
      : [],
  )
  return [...new Set(names)]
}

export function firstWord(name: string | null | undefined) {
  return name?.trim().split(/\s+/)[0] || null
}
