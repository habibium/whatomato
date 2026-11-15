import { useId } from 'react'
import { Field } from '@astryxdesign/core/Field'
import { HStack } from '@astryxdesign/core/HStack'
import { NumberInput } from '@astryxdesign/core/NumberInput'
import {
  SegmentedControl,
  SegmentedControlItem,
} from '@astryxdesign/core/SegmentedControl'
import { TextArea } from '@astryxdesign/core/TextArea'

import { NOTE_MAX, SEND_LABELS } from '@/lib/agent-grant'
import type { SendLevel } from '@/lib/agent-grant'

export const NOTE_HINT =
  'Guidance the agent reads, such as “reply in Bangla”. It isn’t enforced.'

interface SendControlProps {
  label: string
  isLabelHidden?: boolean
  description?: string
  value: SendLevel
  onChange: (value: SendLevel) => void
  /** Shown attached under the control, at the point the risky choice is made. */
  warning?: string | null
  size?: 'sm' | 'md'
}

export function SendControl({
  label,
  isLabelHidden,
  description,
  value,
  onChange,
  warning,
  size = 'md',
}: SendControlProps) {
  const id = useId()
  return (
    <Field
      label={label}
      inputID={id}
      isGroupLabel
      isLabelHidden={isLabelHidden}
      description={description}
      status={warning ? { type: 'warning', message: warning } : undefined}
    >
      <HStack>
        <SegmentedControl
          label={label}
          size={size}
          value={value}
          onChange={(next) => onChange(next as SendLevel)}
        >
          {(['no', 'ask', 'yes'] as const).map((level) => (
            <SegmentedControlItem
              key={level}
              value={level}
              label={SEND_LABELS[level]}
            />
          ))}
        </SegmentedControl>
      </HStack>
    </Field>
  )
}

interface DailyLimitFieldProps {
  label: string
  isLabelHidden?: boolean
  description?: string
  value: number | null
  onChange: (value: number | null) => void
  isDisabled?: boolean
  size?: 'sm' | 'md'
  width?: string | number
}

export function DailyLimitField({
  label,
  isLabelHidden,
  description,
  value,
  onChange,
  isDisabled,
  size = 'md',
  width,
}: DailyLimitFieldProps) {
  return (
    <NumberInput
      label={label}
      isLabelHidden={isLabelHidden}
      description={description}
      isOptional={!isLabelHidden}
      size={size}
      width={width}
      placeholder="No limit"
      units="a day"
      min={1}
      max={10_000}
      isIntegerOnly
      hasClear
      isWheelEnabled={false}
      isDisabled={isDisabled}
      disabledMessage={
        isDisabled
          ? 'It can’t send here, so there’s nothing to limit.'
          : undefined
      }
      value={value}
      onChange={(next: number | null) => onChange(next ?? null)}
    />
  )
}

interface NoteFieldProps {
  label: string
  isLabelHidden?: boolean
  value: string | null
  onChange: (value: string | null) => void
  size?: 'sm' | 'md'
  hasAutoFocus?: boolean
}

export function NoteField({
  label,
  isLabelHidden,
  value,
  onChange,
  size = 'md',
  hasAutoFocus,
}: NoteFieldProps) {
  return (
    <TextArea
      label={label}
      isLabelHidden={isLabelHidden}
      description={NOTE_HINT}
      isOptional
      size={size}
      rows={2}
      maxLength={NOTE_MAX}
      hasAutoFocus={hasAutoFocus}
      value={value ?? ''}
      onChange={(next) => onChange(next === '' ? null : next)}
    />
  )
}
