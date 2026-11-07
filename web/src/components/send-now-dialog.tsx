import { useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { useForm, useStore } from '@tanstack/react-form'
import { Banner } from '@astryxdesign/core/Banner'
import { Button } from '@astryxdesign/core/Button'
import { Dialog, DialogHeader } from '@astryxdesign/core/Dialog'
import { FormLayout } from '@astryxdesign/core/FormLayout'
import { HStack } from '@astryxdesign/core/HStack'
import { Layout, LayoutContent, LayoutFooter } from '@astryxdesign/core/Layout'
import { VStack } from '@astryxdesign/core/VStack'
import { z } from 'zod'

import { api } from '@/api/client'
import { DialogForm } from '@/components/dialog-form'
import { MessageBodyField } from '@/components/message-body-field'
import { RecipientField, checkRecipient } from '@/components/recipient-field'
import type { Recipient } from '@/components/recipient-field'
import { toFieldErrors, toFieldStatus } from '@/lib/form-errors'
import { useVariableContext } from '@/lib/contacts'
import {
  BODY_MAX,
  BROWSER_TIME_ZONE,
  DELIVERIES_KEY,
  bodySchema,
} from '@/lib/scheduler'
import { digitsOf, formatPhoneNumber } from '@/lib/whatsapp'
import { connectionProblem, toWriteProblem } from '@/lib/write-problem'
import type { WriteProblem } from '@/lib/write-problem'

const FAILED = "Couldn't send the message."

const schema = z
  .object({
    recipient_type: z.enum(['contact', 'group']),
    recipient: z.string(),
    recipient_name: z.string().nullable(),
    body: bodySchema,
  })
  .superRefine(checkRecipient)

type Values = z.infer<typeof schema>

function toValues(recipient: Recipient | null): Values {
  return {
    recipient_type: recipient?.recipient_type ?? 'contact',
    recipient:
      recipient?.recipient_type === 'contact'
        ? formatPhoneNumber(recipient.recipient)
        : (recipient?.recipient ?? ''),
    recipient_name: recipient?.recipient_name ?? null,
    body: '',
  }
}

interface SendNowDialogProps {
  isOpen: boolean
  onOpenChange: (isOpen: boolean) => void
  /** The message is queued, and the dialog has closed. */
  onSent: () => void
  /** Who it goes to, with digits for a number. */
  recipient?: Recipient | null
}

export function SendNowDialog({
  isOpen,
  onOpenChange,
  onSent,
  recipient = null,
}: SendNowDialogProps) {
  return (
    <Dialog
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      purpose="form"
      width={480}
    >
      {isOpen ? (
        <SendNowForm
          recipient={recipient}
          onSent={onSent}
          onClose={() => onOpenChange(false)}
        />
      ) : null}
    </Dialog>
  )
}

function SendNowForm({
  recipient,
  onSent,
  onClose,
}: {
  recipient: Recipient | null
  onSent: () => void
  onClose: () => void
}) {
  const queryClient = useQueryClient()
  const [problem, setProblem] = useState<WriteProblem | null>(null)
  const [isWhatsAppDown, setIsWhatsAppDown] = useState(false)

  const form = useForm({
    defaultValues: toValues(recipient),
    validators: { onSubmit: schema },
    canSubmitWhenInvalid: true,
    listeners: {
      onChange: () => setProblem(null),
    },
    onSubmit: async ({ value, formApi }) => {
      try {
        const result = await api.POST('/api/messages/send-now', {
          body: {
            recipient_type: value.recipient_type,
            recipient:
              value.recipient_type === 'contact'
                ? digitsOf(value.recipient)
                : value.recipient,
            recipient_name: value.recipient_name,
            body: value.body.trim(),
            timezone: BROWSER_TIME_ZONE,
          },
        })
        if (result.data) {
          await queryClient.invalidateQueries({ queryKey: DELIVERIES_KEY })
          onClose()
          onSent()
          return
        }
        const fields = toFieldErrors(result.error, [
          'recipient',
          'body',
        ] as const)
        if (fields) {
          formApi.setErrorMap({ onSubmit: { fields } })
        } else {
          setIsWhatsAppDown(result.response.status === 409)
          setProblem(toWriteProblem(result, FAILED))
        }
      } catch {
        setIsWhatsAppDown(false)
        setProblem(connectionProblem(FAILED))
      }
    },
  })

  const recipientType = useStore(
    form.store,
    (state) => state.values.recipient_type,
  )
  const recipientName = useStore(
    form.store,
    (state) => state.values.recipient_name,
  )
  const recipientValue = useStore(form.store, (state) => state.values.recipient)
  const variables = useVariableContext(
    {
      recipient_type: recipientType,
      recipient: recipientValue,
      recipient_name: recipientName,
    },
    null,
    BROWSER_TIME_ZONE,
  )

  return (
    <DialogForm onSubmit={() => void form.handleSubmit()}>
      <Layout
        header={
          <DialogHeader
            title="Send a message now"
            subtitle="It goes out straight away, and shows in History."
            onOpenChange={() => onClose()}
          />
        }
        content={
          <LayoutContent>
            <VStack gap={4}>
              {problem ? (
                <Banner
                  status="error"
                  title={problem.title}
                  description={problem.description}
                  endContent={
                    isWhatsAppDown ? (
                      <Button
                        label="Open WhatsApp"
                        size="sm"
                        href="/whatsapp"
                      />
                    ) : undefined
                  }
                />
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
                      status={toFieldStatus(field.state.meta)}
                    />
                  )}
                </form.Field>
                <form.Field name="body">
                  {(field) => (
                    <MessageBodyField
                      maxLength={BODY_MAX}
                      htmlName={field.name}
                      value={field.state.value}
                      onChange={(value) => field.handleChange(value)}
                      onBlur={field.handleBlur}
                      status={toFieldStatus(field.state.meta)}
                      context={variables}
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
                    label="Send now"
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
