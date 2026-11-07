import { HOUR_FORMAT, describeCron, parseCron } from '@/lib/cron'
import { BROWSER_TIME_ZONE } from '@/lib/scheduler'

const DAY_MS = 86_400_000

const clock = new Intl.DateTimeFormat(undefined, {
  hour: 'numeric',
  minute: '2-digit',
  hourCycle: HOUR_FORMAT === '24h' ? 'h23' : 'h12',
})
const weekdayShort = new Intl.DateTimeFormat(undefined, {
  weekday: 'short',
  day: 'numeric',
  month: 'short',
})
const dayMonth = new Intl.DateTimeFormat(undefined, {
  day: 'numeric',
  month: 'short',
})
const dayMonthYear = new Intl.DateTimeFormat(undefined, {
  day: 'numeric',
  month: 'short',
  year: 'numeric',
})
const weekdayLong = new Intl.DateTimeFormat(undefined, {
  weekday: 'long',
  day: 'numeric',
  month: 'long',
})
const fullDate = new Intl.DateTimeFormat(undefined, {
  weekday: 'long',
  day: 'numeric',
  month: 'long',
  year: 'numeric',
})
const relative = new Intl.RelativeTimeFormat(undefined, { numeric: 'auto' })
const ordinals = new Intl.PluralRules('en', { type: 'ordinal' })
const SUFFIX = { one: 'st', two: 'nd', few: 'rd', other: 'th' } as const

/** Whole calendar days from today to the date, in this device's time zone. */
function daysFromToday(date: Date, now = new Date()) {
  const start = (value: Date) =>
    new Date(value.getFullYear(), value.getMonth(), value.getDate()).getTime()
  return Math.round((start(date) - start(now)) / DAY_MS)
}

export function formatClock(iso: string) {
  return clock.format(new Date(iso))
}

/** "Today", "Tomorrow", "Mon 10 Nov" within a week, then the date; the year only when it isn't this one. */
export function formatDay(iso: string) {
  const date = new Date(iso)
  const days = daysFromToday(date)
  if (Math.abs(days) <= 1) return capitalise(relative.format(days, 'day'))
  if (Math.abs(days) < 7) return weekdayShort.format(date)
  return date.getFullYear() === new Date().getFullYear()
    ? dayMonth.format(date)
    : dayMonthYear.format(date)
}

export function formatWhen(iso: string) {
  return `${formatDay(iso)}, ${formatClock(iso)}`
}

/** For a sentence: "tomorrow at 11:30 PM", "on Mon 9 Nov at 3:45 PM". */
export function formatWhenInSentence(iso: string) {
  const day = formatDay(iso)
  const isRelative = Math.abs(daysFromToday(new Date(iso))) <= 1
  return isRelative
    ? `${day.toLowerCase()} at ${formatClock(iso)}`
    : `on ${day} at ${formatClock(iso)}`
}

/** A day heading in a log: "Today", "Yesterday", then "Tuesday 4 November". */
export function formatDayHeading(iso: string) {
  const date = new Date(iso)
  const days = daysFromToday(date)
  if (days === 0 || days === -1) return capitalise(relative.format(days, 'day'))
  return date.getFullYear() === new Date().getFullYear()
    ? weekdayLong.format(date)
    : fullDate.format(date)
}

/** The local calendar day, for grouping rows under one heading. */
export function dayKey(iso: string) {
  const date = new Date(iso)
  return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`
}

/** "London time" for Europe/London; nothing when it's this device's own zone. */
export function zoneLabel(timeZone: string) {
  if (timeZone === BROWSER_TIME_ZONE) return null
  const city = timeZone.split('/').at(-1)?.replaceAll('_', ' ')
  return city && city !== timeZone ? `${city} time` : timeZone
}

function capitalise(text: string) {
  return text.charAt(0).toUpperCase() + text.slice(1)
}

function ordinal(day: number) {
  const rule = ordinals.select(day) as keyof typeof SUFFIX
  return `${day}${SUFFIX[rule] ?? SUFFIX.other}`
}

const WEEKDAY_NAMES = [
  'Sunday',
  'Monday',
  'Tuesday',
  'Wednesday',
  'Thursday',
  'Friday',
  'Saturday',
]

/** "1-5" in the weekday field as "1,2,3,4,5", so the presets can read it. */
function expandWeekdayRanges(expression: string) {
  const fields = expression.trim().split(/\s+/)
  const weekday = fields[4]
  if (
    fields.length !== 5 ||
    !weekday ||
    !/^[0-6](-[0-6])?(,[0-6](-[0-6])?)*$/.test(weekday)
  ) {
    return expression
  }
  const days = weekday.split(',').flatMap((part) => {
    const [from = 0, to = from] = part.split('-').map(Number)
    return Array.from(
      { length: Math.max(0, to - from + 1) },
      (_, index) => from + index,
    )
  })
  fields[4] = days.join(',')
  return fields.join(' ')
}

function clockOf(time: string) {
  const [hour = 0, minute = 0] = time.split(':').map(Number)
  return clock.format(new Date(2000, 0, 1, hour, minute))
}

/** "Weekdays at 09:45", "Tuesdays at 12:00", "Monthly on the 28th at 09:00"; cronstrue for the rest. */
export function summariseCron(expression: string) {
  const controls = parseCron(expandWeekdayRanges(expression))
  const at = clockOf(controls.time)
  switch (controls.frequency) {
    case 'daily':
      return `Every day at ${at}`
    case 'monthly':
      return `Monthly on the ${ordinal(controls.monthDay)} at ${at}`
    case 'weekly': {
      const days = [...controls.weekdays].sort((a, b) => a - b)
      const key = days.join(',')
      if (key === '1,2,3,4,5') return `Weekdays at ${at}`
      if (key === '0,6') return `Weekends at ${at}`
      if (days.length === 7) return `Every day at ${at}`
      if (days.length === 1) return `${WEEKDAY_NAMES[days[0] ?? 0]}s at ${at}`
      const names = days.map((day) => WEEKDAY_NAMES[day]?.slice(0, 3))
      return `${names.join(', ')} at ${at}`
    }
    case 'custom':
      return describeCron(expression) ?? expression
  }
}
