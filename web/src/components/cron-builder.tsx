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

import {
  HOUR_FORMAT,
  LAST_MONTH_DAY,
  WEEKDAYS,
  buildCron,
  describeCron,
  parseCron,
} from '@/lib/cron'
import type { CronControls, Frequency } from '@/lib/cron'

const FREQUENCIES = [
  { value: 'daily', label: 'Daily' },
  { value: 'weekly', label: 'Weekly' },
  { value: 'monthly', label: 'Monthly' },
  { value: 'custom', label: 'Custom cron' },
]

const MONTH_DAYS = Array.from({ length: LAST_MONTH_DAY }, (_, index) =>
  String(index + 1),
)

const styles = stylex.create({
  // Equal cells, so the row keeps to the field edges and still fits a phone
  weekdays: {
    display: 'grid',
    gridTemplateColumns: 'repeat(7, minmax(0, 1fr))',
  },
  weekday: { paddingInline: 0 },
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
  const description = cron ? describeCron(cron) : null

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
                label={day.label}
                xstyle={styles.weekday}
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
          onChange={(day) => update({ monthDay: Number(day) })}
        />
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

      <Text aria-live="polite" color={description ? 'primary' : 'secondary'}>
        {controls.frequency === 'weekly' && controls.weekdays.length === 0
          ? 'Pick at least one day.'
          : (description ?? 'That isn’t a schedule yet.')}
      </Text>
      {status?.message && !custom ? (
        <FieldStatus type="error" message={status.message} variant="detached" />
      ) : null}
    </VStack>
  )
}
