import { clock, describeRecurrence } from '@/lib/schedule'
import type { Recurrence } from '@/lib/schedule'
import { BROWSER_TIME_ZONE } from '@/lib/scheduler'

const DAY_MS = 86_400_000

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

/** "Monday to Thursday at 9:00 AM", "Every day, 10 minutes before sunrise"; cronstrue for the rest. */
export function summariseRecurrence(recurrence: Recurrence | null) {
  if (!recurrence) return ''
  const described = describeRecurrence(recurrence)
  if (described) return described
  return recurrence.type === 'cron'
    ? recurrence.expression
    : `${capitalise(recurrence.event)}, on ${recurrence.days}`
}
