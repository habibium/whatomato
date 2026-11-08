import { useId, useState } from 'react'
import * as stylex from '@stylexjs/stylex'
import { Field } from '@astryxdesign/core/Field'
import { FieldStatus } from '@astryxdesign/core/FieldStatus'
import { FormLayout } from '@astryxdesign/core/FormLayout'
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

import {
  HOUR_FORMAT,
  NTHS,
  WEEKDAYS,
  buildCron,
  daysInMonth,
  describeControls,
  parseCron,
  scheduleNote,
} from '@/lib/cron'
import type { CronControls, Frequency, Nth } from '@/lib/cron'

const FREQUENCIES = [
  { value: 'weekly', label: 'Days of the week' },
  { value: 'monthly', label: 'Day of the month' },
  { value: 'nth-weekday', label: 'Weekday of the month' },
  { value: 'yearly', label: 'Date each year' },
  { value: 'custom', label: 'Custom cron' },
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
})

interface CronBuilderProps {
  value: string
  onChange: (cron: string) => void
  status?: { type: 'error'; message?: string }
}

export function CronBuilder({ value, onChange, status }: CronBuilderProps) {
  const id = useId()
  const [controls, setControls] = useState(() => parseCron(value))
  const cron = buildCron(controls)
  const description = cron ? describeControls(controls) : null
  const note = scheduleNote(controls)

  const update = (patch: Partial<CronControls>) => {
    const next = { ...controls, ...patch }
    setControls(next)
    onChange(buildCron(next))
  }

  const custom = controls.frequency === 'custom'

  return (
    <VStack gap={3}>
      <FormLayout direction="horizontal">
        <Selector
          label="Repeats"
          size="lg"
          options={FREQUENCIES}
          value={controls.frequency}
          onChange={(frequency) =>
            update({
              frequency: frequency as Frequency,
              // So switching to custom starts from the schedule on show
              custom: cron || controls.custom,
            })
          }
        />
        {custom ? null : (
          <TimeInput
            label="At"
            size="lg"
            hourFormat={HOUR_FORMAT}
            value={controls.time as ISOTimeString}
            onChange={(time) => {
              if (time) update({ time })
            }}
          />
        )}
      </FormLayout>

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
      ) : null}

      <VStack gap={1}>
        <Text aria-live="polite" color={description ? 'primary' : 'secondary'}>
          {controls.frequency === 'weekly' && controls.weekdays.length === 0
            ? 'Pick at least one day.'
            : (description ?? 'That isn’t a schedule yet.')}
        </Text>
        {note ? <Text type="supporting">{note}</Text> : null}
      </VStack>
      {status?.message && !custom ? (
        <FieldStatus type="error" message={status.message} variant="detached" />
      ) : null}
    </VStack>
  )
}
