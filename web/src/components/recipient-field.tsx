import { useId, useRef } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Button } from '@astryxdesign/core/Button'
import { Field } from '@astryxdesign/core/Field'
import {
  SegmentedControl,
  SegmentedControlItem,
} from '@astryxdesign/core/SegmentedControl'
import { Selector } from '@astryxdesign/core/Selector'
import { VStack } from '@astryxdesign/core/VStack'
import type { z } from 'zod'

import { PhoneNumberField } from '@/components/phone-number-field'
import type { RecipientType } from '@/lib/scheduler'
import {
  ApiError,
  NUMBER_HINT,
  groupsQuery,
  isPhoneNumber,
} from '@/lib/whatsapp'

/** Past this many groups, scrolling for one takes longer than typing its name. */
const SEARCHABLE_FROM = 8

interface Recipient {
  recipient_type: RecipientType
  recipient: string
  recipient_name: string | null
}

export function checkRecipient(
  { recipient_type, recipient }: Recipient,
  context: z.RefinementCtx,
) {
  const message =
    recipient_type === 'group'
      ? !recipient && 'Choose a group'
      : !isPhoneNumber(recipient) && NUMBER_HINT
  if (message) {
    context.addIssue({ code: 'custom', path: ['recipient'], message })
  }
}

interface RecipientFieldProps {
  value: Recipient
  onChange: (value: Recipient) => void
  onBlur: () => void
  status?: { type: 'error'; message?: string }
}

export function RecipientField({
  value,
  onChange,
  onBlur,
  status,
}: RecipientFieldProps) {
  const id = useId()
  const previous = useRef<Recipient | null>(null)

  // Flipping back brings the other choice back: a saved group may have no list to pick it from
  const switchTo = (type: RecipientType) => {
    const restored = previous.current
    previous.current = value
    onChange(
      restored?.recipient_type === type
        ? restored
        : { recipient_type: type, recipient: '', recipient_name: null },
    )
  }

  return (
    <VStack gap={2}>
      <Field label="Send to" inputID={id} isGroupLabel>
        <SegmentedControl
          label="Send to"
          layout="fill"
          size="lg"
          value={value.recipient_type}
          onChange={(type) => switchTo(type as RecipientType)}
        >
          <SegmentedControlItem value="contact" label="Phone number" />
          <SegmentedControlItem value="group" label="Group" />
        </SegmentedControl>
      </Field>
      {value.recipient_type === 'contact' ? (
        <PhoneNumberField
          label="Phone number"
          isLabelHidden
          placeholder="+44 7700 900123"
          htmlName="recipient"
          value={value.recipient}
          onChange={(recipient) =>
            onChange({ ...value, recipient, recipient_name: null })
          }
          onBlur={onBlur}
          isInvalid={status !== undefined}
          errorMessage={status?.message}
        />
      ) : (
        <GroupPicker value={value} onChange={onChange} status={status} />
      )}
    </VStack>
  )
}

const NOT_CONNECTED =
  "WhatsApp isn't connected, so your groups can't load. Connect it on the WhatsApp page, or send to a phone number."
const NOT_CONNECTED_SAVED =
  "WhatsApp isn't connected, so this is the group you saved. Connect it on the WhatsApp page to choose another."

function GroupPicker({
  value,
  onChange,
  status,
}: Pick<RecipientFieldProps, 'value' | 'onChange' | 'status'>) {
  const groups = useQuery(groupsQuery)
  const isNotConnected =
    groups.error instanceof ApiError && groups.error.status === 409
  const list = groups.data ?? []
  const options = list.map((group) => ({
    value: group.id,
    label: group.name || 'Unnamed group',
  }))
  // A saved group may not be on the list: it can't load, or the number has left
  if (value.recipient && !list.some((group) => group.id === value.recipient)) {
    options.unshift({
      value: value.recipient,
      label: value.recipient_name || 'Unnamed group',
    })
  }

  const warning = isNotConnected
    ? value.recipient
      ? NOT_CONNECTED_SAVED
      : NOT_CONNECTED
    : groups.isError
      ? "Couldn't load your groups."
      : undefined

  return (
    <VStack gap={2} hAlign="start">
      <Selector
        label="Group"
        isLabelHidden
        size="lg"
        width="100%"
        placeholder="Choose a group"
        emptyText="This number isn't in any groups yet."
        options={options}
        value={value.recipient || undefined}
        onChange={(id) =>
          onChange({
            ...value,
            recipient: id,
            recipient_name: list.find((group) => group.id === id)?.name || null,
          })
        }
        isLoading={groups.isPending}
        isReadOnly={groups.isError}
        hasSearch={options.length > SEARCHABLE_FROM}
        status={
          status ??
          (warning ? { type: 'warning', message: warning } : undefined)
        }
      />
      {groups.isError && !isNotConnected ? (
        <Button
          label="Try again"
          size="sm"
          clickAction={async () => {
            await groups.refetch()
          }}
        />
      ) : null}
    </VStack>
  )
}
