import cronstrue from 'cronstrue'

import type { components } from '@/api/schema'

export type Recurrence = components['schemas']['Recurrence']
export type Place = components['schemas']['Place']
export type SunEvent = components['schemas']['SunEvent']

export const HOUR_FORMAT = new Intl.DateTimeFormat(undefined, {
  hour: 'numeric',
}).resolvedOptions().hour12
  ? '12h'
  : '24h'

export const clock = new Intl.DateTimeFormat(undefined, {
  hour: 'numeric',
  minute: '2-digit',
  hourCycle: HOUR_FORMAT === '24h' ? 'h23' : 'h12',
})

export const DEFAULT_RECURRENCE: Recurrence = {
  type: 'cron',
  expression: '0 9 * * *',
}

/** Monday first, as the toggles show them; each value as cron numbers it, Sunday being 0. */
export const WEEKDAYS = [
  { value: 1, short: 'Mon', name: 'Monday' },
  { value: 2, short: 'Tue', name: 'Tuesday' },
  { value: 3, short: 'Wed', name: 'Wednesday' },
  { value: 4, short: 'Thu', name: 'Thursday' },
  { value: 5, short: 'Fri', name: 'Friday' },
  { value: 6, short: 'Sat', name: 'Saturday' },
  { value: 0, short: 'Sun', name: 'Sunday' },
]

export const NTHS = [
  { value: '1', label: 'First' },
  { value: '2', label: 'Second' },
  { value: '3', label: 'Third' },
  { value: '4', label: 'Fourth' },
  { value: 'last', label: 'Last' },
] as const

export type Nth = (typeof NTHS)[number]['value']

export type Frequency =
  | 'weekly'
  | 'monthly'
  | 'nth-weekday'
  | 'yearly'
  | 'custom'

/** What sets the time of day: the clock, or the sun. */
export type Timing = 'clock' | SunEvent

/** In minutes, before the sun when negative: the offsets offered, in the order of the day. */
export const SUN_OFFSETS = [
  -120, -90, -60, -45, -30, -20, -15, -10, -5, 0, 5, 10, 15, 20, 30, 45, 60, 90,
  120,
]

export interface ScheduleControls {
  frequency: Frequency
  timing: Timing
  /** `HH:MM`, 24-hour, for a set time. */
  time: string
  /** Minutes after the sun rises or sets, or before it when negative. */
  offset: number
  /** Where the sun is watched from; `null` for the main city of the time zone. */
  place: Place | null
  /** Every day when all seven are on. */
  weekdays: number[]
  monthDay: number | 'last'
  nth: Nth
  nthWeekday: number
  /** 1 to 12. */
  month: number
  yearDay: number
  custom: string
}

const ALL_DAYS = [0, 1, 2, 3, 4, 5, 6]
const DAY_NAMES = ['SUN', 'MON', 'TUE', 'WED', 'THU', 'FRI', 'SAT']

function defaults(): ScheduleControls {
  const today = new Date()
  return {
    frequency: 'weekly',
    timing: 'clock',
    time: '09:00',
    offset: 0,
    place: null,
    weekdays: ALL_DAYS,
    monthDay: 1,
    nth: '1',
    nthWeekday: 1,
    month: today.getMonth() + 1,
    yearDay: today.getDate(),
    custom: '0 9 * * *',
  }
}

/** In a leap year, so 29 February is offered. */
export function daysInMonth(month: number) {
  return new Date(2000, month, 0).getDate()
}

function numberIn(field: string, min: number, max: number) {
  if (!/^\d{1,2}$/.test(field)) return null
  const value = Number(field)
  return value >= min && value <= max ? value : null
}

function weekdayOf(field: string) {
  const named = DAY_NAMES.indexOf(field.toUpperCase())
  if (named !== -1) return named
  return numberIn(field, 0, 7)
}

/** The days a weekday field names, as lists and ranges of numbers or names; `null` for anything else. */
function weekdaysOf(field: string) {
  const days = new Set<number>()
  for (const part of field.split(',')) {
    const [from, to = from, ...rest] = part.split('-')
    const start = from === undefined ? null : weekdayOf(from)
    const end = weekdayOf(to ?? '')
    if (rest.length > 0 || start === null || end === null || start > end) {
      return null
    }
    // 7 is Sunday too, so 5-7 is Friday to Sunday
    for (let day = start; day <= end; day += 1) days.add(day % 7)
  }
  return [...days]
}

/** Reads an expression back into the controls; anything they can't show exactly stays custom. */
function parseCron(expression: string): ScheduleControls {
  const base = defaults()
  const custom = { ...base, frequency: 'custom' as const, custom: expression }
  const fields = expression
    .trim()
    .split(/\s+/)
    .map((field) => (field === '?' ? '*' : field))
  if (fields.length !== 5) return custom
  const [minute = '', hour = '', day = '', month = '', weekday = ''] = fields
  const minuteValue = numberIn(minute, 0, 59)
  const hourValue = numberIn(hour, 0, 23)
  if (minuteValue === null || hourValue === null) return custom
  const time = `${String(hourValue).padStart(2, '0')}:${String(minuteValue).padStart(2, '0')}`
  const preset = { ...base, time, custom: expression }

  if (month === '*' && day === '*') {
    if (weekday === '*') return { ...preset, frequency: 'weekly' }
    const nth = /^([0-7])(#([1-4])|L)$/i.exec(weekday)
    if (nth) {
      return {
        ...preset,
        frequency: 'nth-weekday',
        nthWeekday: Number(nth[1]) % 7,
        nth: (nth[3] ?? 'last') as Nth,
      }
    }
    const weekdays = weekdaysOf(weekday)
    return weekdays ? { ...preset, frequency: 'weekly', weekdays } : custom
  }
  if (month === '*' && weekday === '*') {
    const monthDay = day.toUpperCase() === 'L' ? 'last' : numberIn(day, 1, 31)
    return monthDay === null
      ? custom
      : { ...preset, frequency: 'monthly', monthDay }
  }
  const monthValue = numberIn(month, 1, 12)
  if (monthValue !== null && weekday === '*') {
    const yearDay = numberIn(day, 1, daysInMonth(monthValue))
    if (yearDay !== null) {
      return { ...preset, frequency: 'yearly', month: monthValue, yearDay }
    }
  }
  return custom
}

/** "1-4" for Monday to Thursday; runs shorter than three stay a list. */
function weekdayField(weekdays: number[]) {
  const days = [...new Set(weekdays)].sort((a, b) => a - b)
  if (days.length === 7) return '*'
  const parts: string[] = []
  let index = 0
  while (index < days.length) {
    let end = index
    while (days[end + 1] === (days[end] ?? 0) + 1) end += 1
    if (end - index >= 2) {
      parts.push(`${days[index]}-${days[end]}`)
    } else {
      for (let each = index; each <= end; each += 1) {
        parts.push(String(days[each]))
      }
    }
    index = end + 1
  }
  return parts.join(',')
}

/** Reads a schedule back into the controls, as `parseCron` does. */
export function parseRecurrence(
  recurrence: Recurrence | null,
): ScheduleControls {
  if (!recurrence) return defaults()
  if (recurrence.type === 'cron') return parseCron(recurrence.expression)
  // The days with a stand-in time, which only shows if the timing goes back to the clock
  const controls = parseCron(`0 9 ${recurrence.days}`)
  if (controls.frequency === 'custom') return controls
  return {
    ...controls,
    timing: recurrence.event,
    offset: recurrence.offset_minutes,
    place: recurrence.place,
  }
}

/** Cron's day fields, day of the month, month and weekday; empty for a weekly one with no days. */
function dayFields(controls: ScheduleControls) {
  switch (controls.frequency) {
    case 'weekly':
      return controls.weekdays.length > 0
        ? `* * ${weekdayField(controls.weekdays)}`
        : ''
    case 'monthly':
      return `${controls.monthDay === 'last' ? 'L' : controls.monthDay} * *`
    case 'nth-weekday':
      return `* * ${controls.nthWeekday}${controls.nth === 'last' ? 'L' : `#${controls.nth}`}`
    case 'yearly':
      return `${controls.yearDay} ${controls.month} *`
    case 'custom':
      return ''
  }
}

/** The controls as a cron expression at the set time; empty when they don't make one yet. */
export function buildCron(controls: ScheduleControls) {
  if (controls.frequency === 'custom') return controls.custom.trim()
  const days = dayFields(controls)
  const [hour = 0, minute = 0] = controls.time.split(':').map(Number)
  return days ? `${minute} ${hour} ${days}` : ''
}

/** `null` when the controls don't make a schedule yet. */
export function buildRecurrence(controls: ScheduleControls): Recurrence | null {
  if (controls.frequency === 'custom' || controls.timing === 'clock') {
    const expression = buildCron(controls)
    return expression ? { type: 'cron', expression } : null
  }
  const days = dayFields(controls)
  return days
    ? {
        type: 'sun',
        event: controls.timing,
        offset_minutes: controls.offset,
        days,
        place: controls.place,
      }
    : null
}

const ordinals = new Intl.PluralRules('en', { type: 'ordinal' })
const SUFFIX = { one: 'st', two: 'nd', few: 'rd', other: 'th' } as const
const list = new Intl.ListFormat('en-GB', { type: 'conjunction' })
const dayAndMonth = new Intl.DateTimeFormat(undefined, {
  day: 'numeric',
  month: 'long',
})
const minutes = new Intl.NumberFormat('en', {
  style: 'unit',
  unit: 'minute',
  unitDisplay: 'long',
})
const hours = new Intl.NumberFormat('en', {
  style: 'unit',
  unit: 'hour',
  unitDisplay: 'long',
})
const tenths = new Intl.NumberFormat(undefined, {
  minimumFractionDigits: 1,
  maximumFractionDigits: 1,
})

export function ordinal(day: number) {
  const rule = ordinals.select(day) as keyof typeof SUFFIX
  return `${day}${SUFFIX[rule] ?? SUFFIX.other}`
}

export function clockOf(time: string) {
  const [hour = 0, minute = 0] = time.split(':').map(Number)
  return clock.format(new Date(2000, 0, 1, hour, minute))
}

/** "10 minutes", "1 hour", "1 hour 30 minutes". */
function duration(total: number) {
  const whole = Math.floor(total / 60)
  const rest = total % 60
  return [whole ? hours.format(whole) : '', rest ? minutes.format(rest) : '']
    .filter(Boolean)
    .join(' ')
}

/** An offset as an option reads: "At sunrise", "10 minutes before", "1 hour after". */
export function offsetLabel(offset: number, event: SunEvent) {
  if (offset === 0) return `At ${event}`
  return `${duration(Math.abs(offset))} ${offset < 0 ? 'before' : 'after'}`
}

/** "23.8° N, 90.4° E". */
export function formatPlace({ latitude, longitude }: Place) {
  const north = `${tenths.format(Math.abs(latitude))}° ${latitude < 0 ? 'S' : 'N'}`
  const east = `${tenths.format(Math.abs(longitude))}° ${longitude < 0 ? 'W' : 'E'}`
  return `${north}, ${east}`
}

/** To a tenth of a degree, about 10 km, as the server keeps it: no closer to someone's home. */
export function roundPlace(latitude: number, longitude: number): Place {
  const round = (degrees: number) => Math.round(degrees * 10) / 10
  return { latitude: round(latitude), longitude: round(longitude) }
}

/** What follows the days: " at 9:00 AM", " at sunset", ", 10 minutes before sunrise". */
function timeOfDay(controls: ScheduleControls) {
  if (controls.timing === 'clock') return ` at ${clockOf(controls.time)}`
  if (controls.offset === 0) return ` at ${controls.timing}`
  const side = controls.offset < 0 ? 'before' : 'after'
  return `, ${duration(Math.abs(controls.offset))} ${side} ${controls.timing}`
}

const nameOf = (day: number) =>
  WEEKDAYS.find((weekday) => weekday.value === day)?.name ?? ''

/** "Every day", "Monday to Thursday", "Mondays and Thursdays", "Mon, Wed, Fri and Sat". */
function describeWeekdays(weekdays: number[]) {
  const on = new Set(weekdays)
  // Monday first, so Saturday to Monday reads as one run across the weekend
  const week = WEEKDAYS.map((day) => day.value)
  const chosen = week.filter((day) => on.has(day))
  if (chosen.length === 7) return 'Every day'
  if (chosen.length === 1) return `Every ${nameOf(chosen[0] ?? 0)}`
  const starts = week.filter(
    (day, index) => on.has(day) && !on.has(week[(index + 6) % 7] ?? 0),
  )
  if (starts.length === 1 && chosen.length >= 3) {
    const start = week.indexOf(starts[0] ?? 0)
    const end = week[(start + chosen.length - 1) % 7] ?? 0
    return `${nameOf(starts[0] ?? 0)} to ${nameOf(end)}`
  }
  if (chosen.length <= 3)
    return list.format(chosen.map((day) => `${nameOf(day)}s`))
  return list.format(
    chosen.map(
      (day) => WEEKDAYS.find((each) => each.value === day)?.short ?? '',
    ),
  )
}

/** The schedule in words, or `null` when the controls don't make one. */
export function describeControls(controls: ScheduleControls) {
  const at = timeOfDay(controls)
  switch (controls.frequency) {
    case 'weekly':
      return controls.weekdays.length > 0
        ? `${describeWeekdays(controls.weekdays)}${at}`
        : null
    case 'monthly':
      return controls.monthDay === 'last'
        ? `Last day of every month${at}`
        : `${ordinal(controls.monthDay)} of every month${at}`
    case 'nth-weekday': {
      const nth = NTHS.find((option) => option.value === controls.nth)?.label
      return `${nth} ${nameOf(controls.nthWeekday)} of every month${at}`
    }
    case 'yearly': {
      const date = new Date(2000, controls.month - 1, controls.yearDay)
      return `Every year on ${dayAndMonth.format(date)}${at}`
    }
    case 'custom':
      return describeCron(controls.custom)
  }
}

/** What the builder can't show on its own: dates that some months or years don't have. */
export function scheduleNote(controls: ScheduleControls) {
  if (
    controls.frequency === 'monthly' &&
    typeof controls.monthDay === 'number'
  ) {
    if (controls.monthDay > 28) {
      return `Months without a ${ordinal(controls.monthDay)} are skipped. Choose Last day to send at the end of every month.`
    }
  }
  if (
    controls.frequency === 'yearly' &&
    controls.month === 2 &&
    controls.yearDay === 29
  ) {
    return 'Only goes out in leap years.'
  }
  return null
}

/** The schedule in words; `null` for what the server wouldn't take either. */
export function describeRecurrence(recurrence: Recurrence) {
  const controls = parseRecurrence(recurrence)
  // Sun days the controls can't show come back as custom cron, whose words would give a clock time
  if (recurrence.type === 'sun' && controls.frequency === 'custom') return null
  return describeControls(controls)
}

/** `null` for what the server wouldn't take either. It takes five fields; cronstrue takes more. */
function describeCron(expression: string) {
  if (expression.trim().split(/\s+/).length !== 5) return null
  try {
    return cronstrue.toString(expression, {
      // Plain "At 9:00 AM" hides that it repeats daily; on a looser schedule it only adds noise
      verbose: /^\s*\d+\s+\d+\s/.test(expression),
      use24HourTimeFormat: HOUR_FORMAT === '24h',
      trimHoursLeadingZero: true,
    })
  } catch {
    return null
  }
}
