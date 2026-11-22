import { useId } from 'react'
import { CheckboxInput } from '@astryxdesign/core/CheckboxInput'
import { Field } from '@astryxdesign/core/Field'
import { HStack } from '@astryxdesign/core/HStack'
import { NumberInput } from '@astryxdesign/core/NumberInput'
import {
  SegmentedControl,
  SegmentedControlItem,
} from '@astryxdesign/core/SegmentedControl'
import { Selector } from '@astryxdesign/core/Selector'
import { TextArea } from '@astryxdesign/core/TextArea'

import {
  DEFAULT_REACH,
  NOTE_MAX,
  SEND_LABELS,
  describeReach,
} from '@/lib/agent-grant'
import type { Reach, SendLevel } from '@/lib/agent-grant'

export const NOTE_HINT =
  'Guidance the agent reads, such as “reply in Bangla”. It isn’t enforced.'

interface ReadControlProps {
  label: string
  description?: string
  value: boolean
  onChange: (value: boolean) => void
  warning?: string | null
  size?: 'sm' | 'md'
}

export function ReadControl({
  label,
  description,
  value,
  onChange,
  warning,
  size = 'md',
}: ReadControlProps) {
  return (
    <CheckboxInput
      label={label}
      description={description}
      size={size}
      value={value}
      onChange={onChange}
      status={warning ? { type: 'warning', message: warning } : undefined}
    />
  )
}

interface SendControlProps {
  label: string
  isLabelHidden?: boolean
  description?: string
  value: SendLevel
  onChange: (value: SendLevel) => void
  /** Shown under the control, at the point the risky choice is made. */
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
      statusVariant="detached"
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

const REACH_CHOICES: readonly Reach[] = [
  { days: 7 },
  { days: 30 },
  { months: 3 },
  { months: 12 },
  'all',
]

function reachKey(reach: Reach) {
  if (reach === 'all') return 'all'
  return 'days' in reach ? `days:${reach.days}` : `months:${reach.months}`
}

function capitalized(text: string) {
  return text.charAt(0).toUpperCase() + text.slice(1)
}

interface ReachFieldProps {
  value: Reach | undefined
  onChange: (value: Reach) => void
  /** Whether Whatomato keeps only a week, so a longer reach reads no further yet. */
  isKeptForAWeek: boolean
}

export function ReachField({
  value,
  onChange,
  isKeptForAWeek,
}: ReachFieldProps) {
  const current = value ?? DEFAULT_REACH
  // A length set some other way stays choosable
  const choices = REACH_CHOICES.some(
    (reach) => reachKey(reach) === reachKey(current),
  )
    ? REACH_CHOICES
    : [...REACH_CHOICES, current]
  const reachesPastAWeek = !(
    typeof current === 'object' &&
    'days' in current &&
    current.days <= 7
  )
  return (
    <Selector
      label="How far back it may read"
      description="In the chats it may read. Older messages stay out of its reach."
      width={320}
      value={reachKey(current)}
      options={choices.map((reach) => ({
        value: reachKey(reach),
        label: capitalized(describeReach(reach)),
      }))}
      onChange={(key) => {
        const chosen = choices.find((reach) => reachKey(reach) === key)
        if (chosen) onChange(chosen)
      }}
      status={
        isKeptForAWeek && reachesPastAWeek
          ? {
              type: 'warning',
              message:
                'Whatomato keeps only the last 7 days until you choose to keep message history on the WhatsApp page.',
            }
          : undefined
      }
      statusVariant="detached"
    />
  )
}
