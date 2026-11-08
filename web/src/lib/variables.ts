/**
 * `{{first_name}}` and the like in a message body. The server fills them in, both as a message is
 * queued and for the preview (`server/src/scheduler/variables.rs`); this side offers them while
 * typing and checks them the same way, worded as the server words it.
 */

interface Variable {
  name: string
  /** What the variable holds, as a phrase that reads after its example. */
  hint: string
  /** Whether it holds a time, which can move: `{{sunset + 10m}}`. */
  isTime: boolean
}

export const VARIABLES: Variable[] = [
  {
    name: 'first_name',
    hint: 'their first name, as saved on your phone',
    isTime: false,
  },
  {
    name: 'name',
    hint: 'their name as saved on your phone, or the group’s name',
    isTime: false,
  },
  { name: 'day', hint: 'the day it goes out', isTime: true },
  { name: 'date', hint: 'the date it goes out', isTime: true },
  { name: 'time', hint: 'the time it goes out', isTime: true },
  { name: 'month', hint: 'the month it goes out', isTime: true },
  { name: 'year', hint: 'the year it goes out', isTime: true },
  { name: 'sunrise', hint: 'sunrise on the day it goes out', isTime: true },
  { name: 'sunset', hint: 'sunset on the day it goes out', isTime: true },
]

const BY_NAME = new Map(VARIABLES.map((variable) => [variable.name, variable]))

export function isTimeVariable(name: string) {
  return BY_NAME.get(name)?.isTime ?? false
}

/** What the server would send for a body, to show while it's written. */
export interface BodyPreview {
  /** As written while the body has a problem. */
  text: string
  problem: string | null
  /** Each variable's value, for its example; `null` where it'd be empty. */
  values: Record<string, string | null>
  /** Variables the body uses that would go out empty, having no fallback. */
  empty: string[]
}

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
 * Puts `name` in place of what's typed. A `}}` already there, with any move or fallback before it,
 * stays, so picking inside `{{sunst + 10m|soon}}` keeps the rest. Returns the text and where the
 * cursor goes.
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
  const close = /^\s*([+-][^|}\n]*)?(\|[^}\n]*)?\}\}/.exec(after)
  const before = `${text.slice(0, open.start)}{{${name}`
  return close
    ? { text: before + after, caret: before.length + close[0].length }
    : { text: `${before}}}${after}`, caret: before.length + 2 }
}

const MINUTES = new Map([
  ...['m', 'min', 'mins', 'minute', 'minutes'].map(
    (unit) => [unit, 1] as const,
  ),
  ...['h', 'hr', 'hrs', 'hour', 'hours'].map((unit) => [unit, 60] as const),
])
const DAYS = new Set(['d', 'day', 'days'])
const SHIFT_LIMIT_DAYS = 366

/** Days and minutes from `+ 10m`, `- 1 hour + 5 minutes`; `null` for anything else. */
function parseShift(text: string) {
  let days = 0
  let minutes = 0
  const term = /^\s*([+-])\s*(\d+)\s*([a-z]*)/i
  let rest = text
  while (rest.trim() !== '') {
    const match = term.exec(rest)
    if (!match) return null
    const [whole, sign, amount = '', unit = ''] = match
    const value = (sign === '-' ? -1 : 1) * Number(amount)
    const lower = unit.toLowerCase()
    const perUnit = MINUTES.get(lower)
    if (DAYS.has(lower)) days += value
    else if (perUnit) minutes += value * perUnit
    else return null
    rest = rest.slice(whole.length)
  }
  return { days, minutes }
}

/** What's wrong with what's between a pair of braces, if anything. */
function innerProblem(inner: string) {
  const bar = inner.indexOf('|')
  const expression = (bar === -1 ? inner : inner.slice(0, bar)).trim()
  const nameEnd = expression.search(/[+-]/)
  const name = (
    nameEnd === -1 ? expression : expression.slice(0, nameEnd)
  ).trim()
  const variable = BY_NAME.get(name)
  if (!variable) {
    return name
      ? `{{${name}}} isn’t a variable`
      : '{{}} needs a variable’s name inside'
  }
  const shift = parseShift(nameEnd === -1 ? '' : expression.slice(nameEnd))
  if (!shift || !Number.isSafeInteger(shift.days + shift.minutes)) {
    return `{{${expression}}} should move by a number and a unit, like + 10m`
  }
  const moves = shift.days !== 0 || shift.minutes !== 0
  if (moves && !variable.isTime) {
    return `{{${name}}} isn’t a time, so it can’t move`
  }
  const total =
    Math.abs(shift.days) + Math.trunc(Math.abs(shift.minutes) / 1440)
  if (total > SHIFT_LIMIT_DAYS) {
    return `{{${expression}}} moves a time by more than a year`
  }
  return null
}

/** What's wrong with a body's variables, if anything, worded as the server words it. */
export function variableProblem(body: string) {
  let rest = body
  for (;;) {
    const open = rest.indexOf('{{')
    if (open === -1) return null
    const close = rest.indexOf('}}', open + 2)
    if (close === -1) return 'A {{ needs a }} to close it'
    const problem = innerProblem(rest.slice(open + 2, close))
    if (problem) return problem
    rest = rest.slice(close + 2)
  }
}

/** Whether a whole `{{…}}` is a variable, moved or not, with or without a fallback. */
export function isVariable(token: string) {
  return innerProblem(token.slice(2, -2)) === null
}

export function hasVariables(body: string) {
  return body.includes('{{') && variableProblem(body) === null
}
