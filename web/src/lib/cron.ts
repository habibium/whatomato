import cronstrue from 'cronstrue'

export const HOUR_FORMAT = new Intl.DateTimeFormat(undefined, {
  hour: 'numeric',
}).resolvedOptions().hour12
  ? '12h'
  : '24h'

export const DEFAULT_CRON = '0 9 * * *'
export const WEEKDAYS = [
  { value: 1, label: 'Mon' },
  { value: 2, label: 'Tue' },
  { value: 3, label: 'Wed' },
  { value: 4, label: 'Thu' },
  { value: 5, label: 'Fri' },
  { value: 6, label: 'Sat' },
  { value: 0, label: 'Sun' },
]
/** Every month has these, so a monthly message never skips one. */
export const LAST_MONTH_DAY = 28

export type Frequency = 'daily' | 'weekly' | 'monthly' | 'custom'

export interface CronControls {
  frequency: Frequency
  time: string
  /** As cron numbers them: 0 is Sunday. */
  weekdays: number[]
  monthDay: number
  custom: string
}

const DEFAULTS: CronControls = {
  frequency: 'daily',
  time: '09:00',
  weekdays: [1],
  monthDay: 1,
  custom: DEFAULT_CRON,
}

function isClockField(field: string, max: number) {
  return /^\d{1,2}$/.test(field) && Number(field) <= max
}

/** Reads an expression back into the controls; anything they can't show exactly stays custom. */
export function parseCron(expression: string): CronControls {
  const custom = {
    ...DEFAULTS,
    frequency: 'custom' as const,
    custom: expression,
  }
  const fields = expression.trim().split(/\s+/)
  if (fields.length !== 5) return custom
  const [minute, hour, day, month, weekday] = fields as [
    string,
    string,
    string,
    string,
    string,
  ]
  const preset = { ...DEFAULTS, custom: expression }
  if (month !== '*' || !isClockField(minute, 59) || !isClockField(hour, 23)) {
    return custom
  }
  const time = `${hour.padStart(2, '0')}:${minute.padStart(2, '0')}`

  if (day === '*' && weekday === '*') {
    return { ...preset, frequency: 'daily', time }
  }
  if (day === '*' && /^[0-6](,[0-6])*$/.test(weekday)) {
    const weekdays = [...new Set(weekday.split(',').map(Number))]
    return { ...preset, frequency: 'weekly', time, weekdays }
  }
  const monthDay = Number(day)
  if (
    weekday === '*' &&
    /^\d{1,2}$/.test(day) &&
    monthDay >= 1 &&
    monthDay <= LAST_MONTH_DAY
  ) {
    return { ...preset, frequency: 'monthly', time, monthDay }
  }
  return custom
}

/** Empty when the controls don't make a schedule yet: a weekly one with no days. */
export function buildCron(controls: CronControls) {
  if (controls.frequency === 'custom') return controls.custom.trim()
  const [hour = 0, minute = 0] = controls.time.split(':').map(Number)
  switch (controls.frequency) {
    case 'daily':
      return `${minute} ${hour} * * *`
    case 'weekly':
      return controls.weekdays.length > 0
        ? `${minute} ${hour} * * ${[...controls.weekdays].sort((a, b) => a - b).join(',')}`
        : ''
    case 'monthly':
      return `${minute} ${hour} ${controls.monthDay} * *`
  }
}

/** `null` for what the server wouldn't take either. It takes five fields; cronstrue takes more. */
export function describeCron(expression: string) {
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
