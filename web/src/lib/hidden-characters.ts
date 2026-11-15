/** What a mark stands for, so the notice can count them by kind. */
export type HiddenKind =
  | 'trailing-space'
  | 'tab'
  | 'unusual-space'
  | 'invisible'
  | 'direction'
  | 'blank-lines'

export type Segment =
  | { kind: 'text'; text: string }
  | { kind: 'newline' }
  | {
      kind: 'mark'
      type: HiddenKind
      /** What is drawn in its place. */
      symbol: string
      /** What a screen reader hears, and the tooltip. */
      label: string
    }

export interface RevealedText {
  segments: Segment[]
  /** How many of each kind; empty when the text holds nothing unusual. */
  counts: Partial<Record<HiddenKind, number>>
  total: number
}

/** Blank lines in a row past this are drawn as one mark, so they can't push text out of sight. */
export const BLANK_RUN = 2

const UNUSUAL_SPACES: Record<string, string> = {
  ' ': 'No-break space',
  ' ': 'Ogham space',
  ' ': 'En quad',
  ' ': 'Em quad',
  ' ': 'En space',
  ' ': 'Em space',
  ' ': 'Three-per-em space',
  ' ': 'Four-per-em space',
  ' ': 'Six-per-em space',
  ' ': 'Figure space',
  ' ': 'Punctuation space',
  ' ': 'Thin space',
  ' ': 'Hair space',
  ' ': 'Narrow no-break space',
  ' ': 'Medium mathematical space',
  '　': 'Ideographic space',
}

const INVISIBLE: Record<string, string> = {
  '­': 'Soft hyphen',
  '͏': 'Combining grapheme joiner',
  ᅟ: 'Hangul filler',
  ᅠ: 'Hangul filler',
  '᠎': 'Mongolian vowel separator',
  '​': 'Zero-width space',
  '‌': 'Zero-width non-joiner',
  '‍': 'Zero-width joiner',
  '⁠': 'Word joiner',
  '⁡': 'Invisible function',
  '⁢': 'Invisible times',
  '⁣': 'Invisible separator',
  '⁤': 'Invisible plus',
  ㅤ: 'Hangul filler',
  '﻿': 'Zero-width no-break space',
  ﾠ: 'Hangul filler',
  '\r': 'Carriage return',
  '\v': 'Vertical tab',
  '\f': 'Form feed',
  '\u0085': 'Next line',
  ' ': 'Line separator',
  ' ': 'Paragraph separator',
}

const DIRECTION: Record<string, string> = {
  '؜': 'Arabic letter mark',
  '‎': 'Left-to-right mark',
  '‏': 'Right-to-left mark',
  '‪': 'Left-to-right embedding',
  '‫': 'Right-to-left embedding',
  '‬': 'Pop directional formatting',
  '‭': 'Left-to-right override',
  '‮': 'Right-to-left override',
  '⁦': 'Left-to-right isolate',
  '⁧': 'Right-to-left isolate',
  '⁨': 'First strong isolate',
  '⁩': 'Pop directional isolate',
}

// Emoji sequences legitimately use ZWJ and variation selectors; tag characters hide ASCII text
const TAG = /[\u{E0000}-\u{E007F}]/u

function hex(char: string) {
  return `U+${(char.codePointAt(0) ?? 0).toString(16).toUpperCase().padStart(4, '0')}`
}

function classify(
  char: string,
): Omit<Extract<Segment, { kind: 'mark' }>, 'kind'> | null {
  if (char === '\t') return { type: 'tab', symbol: '→', label: 'Tab' }
  const space = UNUSUAL_SPACES[char]
  if (space) return { type: 'unusual-space', symbol: '⍽', label: space }
  const direction = DIRECTION[char]
  if (direction)
    return { type: 'direction', symbol: hex(char), label: direction }
  const invisible = INVISIBLE[char]
  if (invisible)
    return { type: 'invisible', symbol: hex(char), label: invisible }
  if (TAG.test(char))
    return { type: 'invisible', symbol: hex(char), label: 'Tag character' }
  return null
}

/** Splits a message into text and visible marks for every character a reader couldn't otherwise see. */
export function revealHidden(body: string): RevealedText {
  const segments: Segment[] = []
  const counts: Partial<Record<HiddenKind, number>> = {}
  const count = (type: HiddenKind) => {
    counts[type] = (counts[type] ?? 0) + 1
  }
  const pushText = (text: string) => {
    if (!text) return
    const last = segments.at(-1)
    if (last?.kind === 'text') last.text += text
    else segments.push({ kind: 'text', text })
  }
  const mark = (segment: Omit<Extract<Segment, { kind: 'mark' }>, 'kind'>) => {
    count(segment.type)
    segments.push({ kind: 'mark', ...segment })
  }

  const lines = body.split('\n')
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index] ?? ''
    if (index > 0) segments.push({ kind: 'newline' })

    // A run of blank lines between text becomes one mark; a single blank line is ordinary
    if (line.trim() === '' && !/[^ ]/.test(line)) {
      let end = index
      while (end + 1 < lines.length && /^ *$/.test(lines[end + 1] ?? ''))
        end += 1
      const run = end - index + 1
      if (run > BLANK_RUN) {
        mark({
          type: 'blank-lines',
          symbol: `${run} blank lines`,
          label: `${run} blank lines`,
        })
        index = end
        continue
      }
    }

    const trailing = /[ \t ]+$/.exec(line)
    const content = trailing ? line.slice(0, trailing.index) : line
    for (const char of content) {
      const found = classify(char)
      if (found) mark(found)
      else pushText(char)
    }
    if (trailing) {
      for (const char of trailing[0]) {
        if (char === ' ')
          mark({ type: 'trailing-space', symbol: '·', label: 'Trailing space' })
        else
          mark(
            classify(char) ?? {
              type: 'trailing-space',
              symbol: '·',
              label: 'Trailing space',
            },
          )
      }
    }
  }

  const total = Object.values(counts).reduce((sum, value) => sum + value, 0)
  return { segments, counts, total }
}

const KIND_WORDS: Record<HiddenKind, [string, string]> = {
  'trailing-space': [
    'space at the end of a line',
    'spaces at the ends of lines',
  ],
  tab: ['tab', 'tabs'],
  'unusual-space': ['unusual space', 'unusual spaces'],
  invisible: ['invisible character', 'invisible characters'],
  direction: ['text-direction control', 'text-direction controls'],
  'blank-lines': ['run of blank lines', 'runs of blank lines'],
}

/** "2 tabs, 1 invisible character and 1 run of blank lines" */
export function describeHidden(counts: RevealedText['counts']) {
  const parts = (Object.keys(KIND_WORDS) as HiddenKind[])
    .filter((kind) => counts[kind])
    .map((kind) => {
      const value = counts[kind] ?? 0
      return `${value} ${KIND_WORDS[kind][value === 1 ? 0 : 1]}`
    })
  if (parts.length <= 1) return parts.join('')
  return `${parts.slice(0, -1).join(', ')} and ${parts.at(-1)}`
}
