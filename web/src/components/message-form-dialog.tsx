import { useEffect, useRef, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useForm, useStore } from '@tanstack/react-form'
import { Banner } from '@astryxdesign/core/Banner'
import { Button } from '@astryxdesign/core/Button'
import { DateTimeInput } from '@astryxdesign/core/DateTimeInput'
import type { ISODateTimeString } from '@astryxdesign/core/DateTimeInput'
import { Dialog, DialogHeader } from '@astryxdesign/core/Dialog'
import { Field } from '@astryxdesign/core/Field'
import { FormLayout } from '@astryxdesign/core/FormLayout'
import { HStack } from '@astryxdesign/core/HStack'
import { Layout, LayoutContent, LayoutFooter } from '@astryxdesign/core/Layout'
import {
  SegmentedControl,
  SegmentedControlItem,
} from '@astryxdesign/core/SegmentedControl'
import { Selector } from '@astryxdesign/core/Selector'
import { Switch } from '@astryxdesign/core/Switch'
import { Text } from '@astryxdesign/core/Text'
import { TextArea } from '@astryxdesign/core/TextArea'
import { VStack } from '@astryxdesign/core/VStack'
import { z } from 'zod'

import type { components } from '@/api/schema'

import { api } from '@/api/client'
import { CronBuilder } from '@/components/cron-builder'
import { DialogForm } from '@/components/dialog-form'
import { RecipientField, checkRecipient } from '@/components/recipient-field'
import { DEFAULT_CRON, HOUR_FORMAT } from '@/lib/cron'
import { toFieldErrors, toFieldStatus } from '@/lib/form-errors'
import {
  BODY_MAX,
  BROWSER_TIME_ZONE,
  bodySchema,
  messagesQuery,
  templatesQuery,
} from '@/lib/scheduler'
import type { ScheduledMessage } from '@/lib/scheduler'
import { digitsOf, formatPhoneNumber } from '@/lib/whatsapp'
import { connectionProblem, toWriteProblem } from '@/lib/write-problem'
import type { WriteProblem } from '@/lib/write-problem'

const FIELDS = [
  'recipient',
  'recipient_name',
  'body',
  'run_at',
  'cron',
  'timezone',
  'template_id',
] as const

const schema = z
  .object({
    recipient_type: z.enum(['contact', 'group']),
    recipient: z.string(),
    recipient_name: z.string().nullable(),
    template_id: z.string().nullable(),
    body: bodySchema,
    schedule_kind: z.enum(['once', 'recurring']),
    run_at: z.string(),
    cron: z.string(),
    enabled: z.boolean(),
  })
  .superRefine((values, context) => {
    checkRecipient(values, context)
    if (values.schedule_kind === 'recurring') {
      if (!values.cron) {
        context.addIssue({
          code: 'custom',
          path: ['cron'],
          message: 'Set a schedule',
        })
      }
    } else if (!(new Date(values.run_at).getTime() > Date.now())) {
      context.addIssue({
        code: 'custom',
        path: ['run_at'],
        message: 'Pick a time in the future',
      })
    }
  })

type Values = z.infer<typeof schema>

/** The `YYYY-MM-DDTHH:MM` of an instant on this device's clock, as `DateTimeInput` takes it. */
function toLocalInput(date: Date) {
  const offset = date.getTimezoneOffset() * 60_000
  return new Date(date.getTime() - offset).toISOString().slice(0, 16)
}

function toValues(message: ScheduledMessage | null): Values {
  return {
    recipient_type: message?.recipient_type ?? 'contact',
    recipient:
      message?.recipient_type === 'contact'
        ? formatPhoneNumber(message.recipient)
        : (message?.recipient ?? ''),
    recipient_name: message?.recipient_name ?? null,
    template_id: message?.template_id ?? null,
    body: message?.body ?? '',
    schedule_kind: message?.schedule_kind ?? 'once',
    run_at: message?.run_at ? toLocalInput(new Date(message.run_at)) : '',
    cron: message?.cron ?? DEFAULT_CRON,
    enabled: message?.enabled ?? true,
  }
}

function toInput(
  values: Values,
  timezone: string,
): components['schemas']['MessageInput'] {
  const isOnce = values.schedule_kind === 'once'
  return {
    recipient_type: values.recipient_type,
    recipient:
      values.recipient_type === 'contact'
        ? digitsOf(values.recipient)
        : values.recipient,
    recipient_name: values.recipient_name,
    template_id: values.template_id,
    body: values.body.trim(),
    schedule_kind: values.schedule_kind,
    run_at: isOnce ? new Date(values.run_at).toISOString() : null,
    cron: isOnce ? null : values.cron,
    timezone,
    enabled: values.enabled,
  }
}

interface MessageFormDialogProps {
  isOpen: boolean
  onOpenChange: (isOpen: boolean) => void
  /** The message to edit; `null` writes a new one. */
  message: ScheduledMessage | null
}

export function MessageFormDialog({
  isOpen,
  onOpenChange,
  message,
}: MessageFormDialogProps) {
  return (
    <Dialog
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      purpose="form"
      width={520}
      maxHeight="90dvh"
    >
      {isOpen ? (
        <MessageForm message={message} onClose={() => onOpenChange(false)} />
      ) : null}
    </Dialog>
  )
}

function MessageForm({
  message,
  onClose,
}: {
  message: ScheduledMessage | null
  onClose: () => void
}) {
  const queryClient = useQueryClient()
  const templates = useQuery(templatesQuery)
  const [problem, setProblem] = useState<WriteProblem | null>(null)
  const formRef = useRef<HTMLFormElement>(null)
  const bannerRef = useRef<HTMLDivElement>(null)
  // A message keeps the zone it was written in; cron reads in that zone wherever it's edited
  const timezone = message?.timezone ?? BROWSER_TIME_ZONE
  const failed = message
    ? "Couldn't save your changes."
    : "Couldn't schedule the message."

  const form = useForm({
    defaultValues: toValues(message),
    validators: { onSubmit: schema },
    canSubmitWhenInvalid: true,
    listeners: {
      onChange: () => setProblem(null),
    },
    onSubmitInvalid: () => revealInvalid(formRef.current),
    onSubmit: async ({ value, formApi }) => {
      const body = toInput(value, timezone)
      try {
        const result = message
          ? await api.PUT('/api/messages/{id}', {
              params: { path: { id: message.id } },
              body,
            })
          : await api.POST('/api/messages', { body })
        await queryClient.invalidateQueries({
          queryKey: messagesQuery.queryKey,
        })
        if (result.data) {
          onClose()
          return
        }
        const fields = toFieldErrors(result.error, FIELDS)
        if (fields) {
          // Neither has a field of its own to carry the message
          const hidden = fields.timezone ?? fields.recipient_name
          if (hidden?.[0]) {
            setProblem({ title: failed, description: hidden[0].message })
          }
          formApi.setErrorMap({ onSubmit: { fields } })
          revealInvalid(formRef.current)
        } else {
          setProblem(toWriteProblem(result, failed))
        }
      } catch {
        setProblem(connectionProblem(failed))
      }
    },
  })

  useEffect(() => {
    if (problem) bannerRef.current?.scrollIntoView({ block: 'nearest' })
  }, [problem])

  const scheduleKind = useStore(
    form.store,
    (state) => state.values.schedule_kind,
  )
  const recipientType = useStore(
    form.store,
    (state) => state.values.recipient_type,
  )
  const recipientName = useStore(
    form.store,
    (state) => state.values.recipient_name,
  )
  const templateID = useStore(form.store, (state) => state.values.template_id)
  const list = templates.data ?? []

  return (
    <DialogForm ref={formRef} onSubmit={() => void form.handleSubmit()}>
      <Layout
        header={
          <DialogHeader
            title={message ? 'Edit message' : 'New message'}
            subtitle="Who it goes to, what it says and when it goes out."
            onOpenChange={() => onClose()}
          />
        }
        content={
          <LayoutContent>
            <VStack gap={4}>
              {problem ? (
                <VStack ref={bannerRef}>
                  <Banner
                    status="error"
                    title={problem.title}
                    description={problem.description}
                  />
                </VStack>
              ) : null}
              <FormLayout defaultOptionality="required">
                <form.Field name="recipient">
                  {(field) => (
                    <RecipientField
                      value={{
                        recipient_type: recipientType,
                        recipient: field.state.value,
                        recipient_name: recipientName,
                      }}
                      onChange={(next) => {
                        form.setFieldValue(
                          'recipient_type',
                          next.recipient_type,
                        )
                        form.setFieldValue(
                          'recipient_name',
                          next.recipient_name,
                        )
                        field.handleChange(next.recipient)
                      }}
                      onBlur={field.handleBlur}
                      status={toFieldStatus(field.state.meta)}
                    />
                  )}
                </form.Field>

                {list.length > 0 || templateID ? (
                  <form.Field name="template_id">
                    {(field) => (
                      <Selector
                        label="Apply template"
                        isOptional
                        size="lg"
                        placeholder="None"
                        hasClear
                        options={list.map((template) => ({
                          value: template.id,
                          label: template.name,
                        }))}
                        value={field.state.value}
                        onChange={(id: string | null) => {
                          field.handleChange(id)
                          const template = list.find((item) => item.id === id)
                          if (template) {
                            form.setFieldValue('body', template.body)
                          }
                        }}
                        status={toFieldStatus(field.state.meta)}
                      />
                    )}
                  </form.Field>
                ) : null}

                <form.Field name="body">
                  {(field) => (
                    <TextArea
                      label="Message"
                      size="lg"
                      rows={5}
                      maxLength={BODY_MAX}
                      htmlName={field.name}
                      value={field.state.value}
                      onChange={(value) => field.handleChange(value)}
                      onBlur={field.handleBlur}
                      status={toFieldStatus(field.state.meta)}
                    />
                  )}
                </form.Field>

                <form.Field name="schedule_kind">
                  {(field) => (
                    <Field label="When" inputID={field.name} isGroupLabel>
                      <SegmentedControl
                        label="When"
                        layout="fill"
                        size="lg"
                        value={field.state.value}
                        onChange={(kind) =>
                          field.handleChange(kind as Values['schedule_kind'])
                        }
                      >
                        <SegmentedControlItem value="once" label="One time" />
                        <SegmentedControlItem
                          value="recurring"
                          label="Recurring"
                        />
                      </SegmentedControl>
                    </Field>
                  )}
                </form.Field>

                {scheduleKind === 'once' ? (
                  <form.Field name="run_at">
                    {(field) => (
                      <VStack gap={2}>
                        <DateTimeInput
                          label="Send at"
                          size="lg"
                          hourFormat={HOUR_FORMAT}
                          value={
                            (field.state.value as ISODateTimeString) ||
                            undefined
                          }
                          onChange={(value) => field.handleChange(value ?? '')}
                          status={toFieldStatus(field.state.meta)}
                        />
                        <Text type="supporting">
                          In your time zone, {BROWSER_TIME_ZONE}.
                        </Text>
                      </VStack>
                    )}
                  </form.Field>
                ) : (
                  <form.Field name="cron">
                    {(field) => (
                      <VStack gap={2}>
                        <CronBuilder
                          value={field.state.value}
                          onChange={(cron) => field.handleChange(cron)}
                          status={toFieldStatus(field.state.meta)}
                        />
                        <Text type="supporting">
                          {timezone === BROWSER_TIME_ZONE
                            ? `In your time zone, ${timezone}.`
                            : `In ${timezone}, the zone it was scheduled in. Yours is ${BROWSER_TIME_ZONE}.`}
                        </Text>
                      </VStack>
                    )}
                  </form.Field>
                )}

                <form.Field name="enabled">
                  {(field) => (
                    <Switch
                      label="Enabled"
                      description="Turn it off to keep the message without sending it."
                      labelPosition="start"
                      labelSpacing="spread"
                      value={field.state.value}
                      onChange={(enabled) => field.handleChange(enabled)}
                    />
                  )}
                </form.Field>
              </FormLayout>
            </VStack>
          </LayoutContent>
        }
        footer={
          <LayoutFooter>
            <HStack gap={2} justify="end">
              <Button
                label="Cancel"
                variant="secondary"
                clickAction={onClose}
              />
              <form.Subscribe selector={(state) => state.isSubmitting}>
                {(isSubmitting) => (
                  <Button
                    type="submit"
                    label={message ? 'Save changes' : 'Schedule message'}
                    variant="primary"
                    isLoading={isSubmitting}
                  />
                )}
              </form.Subscribe>
            </HStack>
          </LayoutFooter>
        }
      />
    </DialogForm>
  )
}

/** A long form scrolls under its pinned footer, so Save can fail on a field that's out of view. */
function revealInvalid(form: HTMLFormElement | null) {
  // After the render that marks the field
  requestAnimationFrame(() => {
    form
      ?.querySelector('[aria-invalid="true"]')
      ?.scrollIntoView({ block: 'center' })
  })
}
