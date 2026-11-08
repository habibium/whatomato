import { useId, useRef, useState } from 'react'
import * as stylex from '@stylexjs/stylex'
import { Clock, LocateFixed, Sunrise, Sunset } from 'lucide-react'
import { Button } from '@astryxdesign/core/Button'
import { Field } from '@astryxdesign/core/Field'
import { FieldStatus } from '@astryxdesign/core/FieldStatus'
import { FormLayout } from '@astryxdesign/core/FormLayout'
import { HStack } from '@astryxdesign/core/HStack'
import { Icon } from '@astryxdesign/core/Icon'
import { Selector } from '@astryxdesign/core/Selector'
import { Text } from '@astryxdesign/core/Text'
import { TextInput } from '@astryxdesign/core/TextInput'
import { TimeInput } from '@astryxdesign/core/TimeInput'
import type { ISOTimeString } from '@astryxdesign/core/TimeInput'
import {
  ToggleButton,
  ToggleButtonGroup,
} from '@astryxdesign/core/ToggleButton'
import { VStack } from '@astryxdesign/core/VStack'
import { colorVars } from '@astryxdesign/core/theme/tokens.stylex'

import { formatWhenInSentence } from '@/lib/format'
import {
  HOUR_FORMAT,
  NTHS,
  SUN_OFFSETS,
  WEEKDAYS,
  buildCron,
  buildRecurrence,
  daysInMonth,
  describeControls,
  formatPlace,
  offsetLabel,
  parseRecurrence,
  roundPlace,
  scheduleNote,
} from '@/lib/schedule'
import type {
  Frequency,
  Nth,
  Recurrence,
  ScheduleControls,
  SunEvent,
  Timing,
} from '@/lib/schedule'
import { useNextRuns } from '@/lib/scheduler'

const FREQUENCIES = [
  { value: 'weekly', label: 'Days of the week' },
  { value: 'monthly', label: 'Day of the month' },
  { value: 'nth-weekday', label: 'Weekday of the month' },
  { value: 'yearly', label: 'Date each year' },
  { value: 'custom', label: 'Custom cron' },
]

const TIMINGS = [
  { value: 'clock', label: 'A set time', icon: Clock },
  { value: 'sunrise', label: 'Sunrise', icon: Sunrise },
  { value: 'sunset', label: 'Sunset', icon: Sunset },
]

const numbers = (count: number) =>
  Array.from({ length: count }, (_, index) => String(index + 1))

const MONTH_DAYS = [
  ...numbers(31).map((day) => ({ value: day, label: day })),
  { value: 'last', label: 'Last day' },
]

const monthName = new Intl.DateTimeFormat(undefined, { month: 'long' })
const MONTHS = numbers(12).map((month) => ({
  value: month,
  label: monthName.format(new Date(2000, Number(month) - 1, 1)),
}))

const WEEKDAY_OPTIONS = WEEKDAYS.map((day) => ({
  value: String(day.value),
  label: day.name,
}))

const list = new Intl.ListFormat('en-GB', { type: 'conjunction' })

/** Before the sun, the moment itself, then after; an offset set elsewhere keeps its place in line. */
function offsetOptions(event: SunEvent, current: number) {
  const offsets = SUN_OFFSETS.includes(current)
    ? SUN_OFFSETS
    : [...SUN_OFFSETS, current].sort((a, b) => a - b)
  const option = (offset: number) => ({
    value: String(offset),
    label: offsetLabel(offset, event),
  })
  const divider = { type: 'divider' as const }
  return [
    ...offsets.filter((offset) => offset < 0).map(option),
    divider,
    option(0),
    divider,
    ...offsets.filter((offset) => offset > 0).map(option),
  ]
}

const LOCATE_PROBLEMS: Record<number, string> = {
  1: 'Your browser didn’t allow it',
  2: 'Your device couldn’t tell where it is',
  3: 'It took too long',
}

function locate(): Promise<GeolocationPosition> {
  return new Promise((resolve, reject) => {
    if (!('geolocation' in navigator)) {
      reject(new Error('This browser can’t share a location'))
      return
    }
    navigator.geolocation.getCurrentPosition(resolve, reject, {
      // A city's worth of accuracy is all the sun needs
      enableHighAccuracy: false,
      maximumAge: 3_600_000,
      timeout: 15_000,
    })
  })
}

/** "Goes out tomorrow at 6:01 AM, then on Tue 10 Nov at 6:02 AM and on Wed 11 Nov at 6:02 AM." */
function describeRuns(runs: string[]) {
  const [first, ...rest] = runs.map(formatWhenInSentence)
  if (!first) return null
  return rest.length > 0
    ? `Goes out ${first}, then ${list.format(rest)}.`
    : `Goes out ${first}.`
}

const styles = stylex.create({
  // Equal cells, so the row keeps to the field edges and still fits a phone
  weekdays: {
    display: 'grid',
    gridTemplateColumns: 'repeat(7, minmax(0, 1fr))',
  },
  weekday: { paddingInline: 0 },
  // The theme's pressed state is a faint grey, too close to off when all seven start on
  chosen: {
    backgroundColor: {
      default: colorVars['--color-accent'],
      '@media (hover: hover)': {
        default: colorVars['--color-accent'],
        ':hover': `color-mix(in srgb, ${colorVars['--color-accent']} 85%, transparent)`,
      },
    },
    color: colorVars['--color-on-accent'],
  },
  place: { flexGrow: 1, flexBasis: 0, minWidth: '12rem' },
})

interface ScheduleBuilderProps {
  value: Recurrence | null
  onChange: (recurrence: Recurrence | null) => void
  /** The zone the days are read in, and whose main city the sun goes by. */
  timezone: string
  status?: { type: 'error'; message?: string }
}

export function ScheduleBuilder({
  value,
  onChange,
  timezone,
  status,
}: ScheduleBuilderProps) {
  const id = useId()
  const [controls, setControls] = useState(() => parseRecurrence(value))
  const [locateProblem, setLocateProblem] = useState<string | null>(null)
  // A location can arrive after other changes; it lands on the controls as they are by then
  const latest = useRef(controls)
  latest.current = controls

  const recurrence = buildRecurrence(controls)
  const description = recurrence ? describeControls(controls) : null
  const note = scheduleNote(controls)
  const next = useNextRuns(description ? recurrence : null, timezone)
  const runs = description ? next.data : undefined

  const update = (patch: Partial<ScheduleControls>) => {
    const changed = { ...latest.current, ...patch }
    latest.current = changed
    setControls(changed)
    onChange(buildRecurrence(changed))
  }

  const shareLocation = async () => {
    try {
      const { coords } = await locate()
      setLocateProblem(null)
      update({ place: roundPlace(coords.latitude, coords.longitude) })
    } catch (error) {
      const code = (error as GeolocationPositionError).code
      const reason = LOCATE_PROBLEMS[code] ?? (error as Error).message
      setLocateProblem(`Couldn’t use your location. ${reason}.`)
    }
  }

  const custom = controls.frequency === 'custom'
  const sun = controls.timing === 'clock' || custom ? null : controls.timing
  const city = runs?.city ?? null
  const place = controls.place
    ? `Times for your location, ${formatPlace(controls.place)}.`
    : city
      ? `Times for ${city}, the main city of the time zone.`
      : null
  // A save's answer, else the preview's; custom cron shows a save's on its own input
  const problem = status?.message
    ? custom
      ? null
      : status.message
    : (runs?.problem ?? null)

  return (
    <VStack gap={3}>
      <Selector
        label="Repeats"
        size="lg"
        options={FREQUENCIES}
        value={controls.frequency}
        onChange={(frequency) =>
          update({
            frequency: frequency as Frequency,
            // So switching to custom starts from the schedule on show
            custom: buildCron(controls) || controls.custom,
          })
        }
      />

      {controls.frequency === 'weekly' ? (
        <Field label="On" inputID={id} isGroupLabel>
          <ToggleButtonGroup
            label="Days of the week"
            type="multiple"
            size="lg"
            xstyle={styles.weekdays}
            value={controls.weekdays.map(String)}
            onChange={(days) =>
              update({ weekdays: (days as string[]).map(Number) })
            }
          >
            {WEEKDAYS.map((day) => (
              <ToggleButton
                key={day.value}
                value={String(day.value)}
                label={day.short}
                xstyle={[
                  styles.weekday,
                  controls.weekdays.includes(day.value) && styles.chosen,
                ]}
              />
            ))}
          </ToggleButtonGroup>
        </Field>
      ) : null}

      {controls.frequency === 'monthly' ? (
        <Selector
          label="On day"
          size="lg"
          options={MONTH_DAYS}
          value={String(controls.monthDay)}
          onChange={(day) =>
            update({ monthDay: day === 'last' ? 'last' : Number(day) })
          }
        />
      ) : null}

      {controls.frequency === 'nth-weekday' ? (
        <FormLayout direction="horizontal">
          <Selector
            label="On the"
            size="lg"
            options={[...NTHS]}
            value={controls.nth}
            onChange={(nth) => update({ nth: nth as Nth })}
          />
          <Selector
            label="Day"
            size="lg"
            options={WEEKDAY_OPTIONS}
            value={String(controls.nthWeekday)}
            onChange={(day) => update({ nthWeekday: Number(day) })}
          />
        </FormLayout>
      ) : null}

      {controls.frequency === 'yearly' ? (
        <FormLayout direction="horizontal">
          <Selector
            label="Month"
            size="lg"
            options={MONTHS}
            value={String(controls.month)}
            onChange={(month) => {
              const days = daysInMonth(Number(month))
              update({
                month: Number(month),
                yearDay: Math.min(controls.yearDay, days),
              })
            }}
          />
          <Selector
            label="Day"
            size="lg"
            options={numbers(daysInMonth(controls.month))}
            value={String(controls.yearDay)}
            onChange={(day) => update({ yearDay: Number(day) })}
          />
        </FormLayout>
      ) : null}

      {custom ? (
        <TextInput
          label="Cron expression"
          size="lg"
          placeholder="0 9 * * 1-5"
          description="Five fields: minute, hour, day of the month, month, day of the week."
          value={controls.custom}
          onChange={(expression) => update({ custom: expression })}
          status={status}
        />
      ) : (
        <FormLayout direction="horizontal">
          <Selector
            label="At"
            size="lg"
            options={TIMINGS}
            value={controls.timing}
            onChange={(timing) => update({ timing: timing as Timing })}
          />
          {sun ? (
            <Selector
              label="Before or after"
              size="lg"
              options={offsetOptions(sun, controls.offset)}
              value={String(controls.offset)}
              onChange={(offset) => update({ offset: Number(offset) })}
            />
          ) : (
            <TimeInput
              label="Time"
              size="lg"
              hourFormat={HOUR_FORMAT}
              value={controls.time as ISOTimeString}
              onChange={(time) => {
                if (time) update({ time })
              }}
            />
          )}
        </FormLayout>
      )}

      {sun ? (
        <HStack gap={2} align="center" justify="between" wrap="wrap">
          {locateProblem || place ? (
            <Text type="supporting" xstyle={styles.place}>
              {locateProblem ?? place}
            </Text>
          ) : null}
          {controls.place && city ? (
            <Button
              label={`Use ${city}`}
              variant="ghost"
              size="sm"
              clickAction={() => {
                setLocateProblem(null)
                update({ place: null })
              }}
            />
          ) : controls.place ? null : (
            <Button
              label="Use my location"
              variant="ghost"
              size="sm"
              icon={<Icon icon={LocateFixed} size="sm" />}
              clickAction={shareLocation}
            />
          )}
        </HStack>
      ) : null}

      <VStack gap={1}>
        <Text aria-live="polite" color={description ? 'primary' : 'secondary'}>
          {controls.frequency === 'weekly' && controls.weekdays.length === 0
            ? 'Pick at least one day.'
            : (description ?? 'That isn’t a schedule yet.')}
        </Text>
        {runs && !runs.problem ? (
          <Text type="supporting">{describeRuns(runs.runs)}</Text>
        ) : null}
        {note ? <Text type="supporting">{note}</Text> : null}
      </VStack>
      {problem ? (
        <FieldStatus type="error" message={problem} variant="detached" />
      ) : null}
    </VStack>
  )
}
