import { useState } from 'react'
import { useForm } from '@tanstack/react-form'
import { Button } from '@astryxdesign/core/Button'
import { Dialog, DialogHeader } from '@astryxdesign/core/Dialog'
import { FormLayout } from '@astryxdesign/core/FormLayout'
import { HStack } from '@astryxdesign/core/HStack'
import { Layout, LayoutContent, LayoutFooter } from '@astryxdesign/core/Layout'
import { VStack } from '@astryxdesign/core/VStack'
import { z } from 'zod'

import { ConfirmProblemBanner } from '@/components/confirm-password-dialog'
import type {
  ConfirmOutcome,
  ConfirmProblem,
} from '@/components/confirm-password-dialog'
import { PasswordField } from '@/components/password-field'
import { PhoneNumberField } from '@/components/phone-number-field'
import { toStatusMessage } from '@/lib/form-errors'

const NUMBER_HINT = 'Enter the whole number, with the country code'

// The server's rule: formatting characters only, and 8 to 15 digits (E.164)
const schema = z.object({
  phone_number: z
    .string()
    .trim()
    .regex(/^\+?[\d\s()-]*$/, NUMBER_HINT)
    .refine((value) => /^\d{8,15}$/.test(digitsOf(value)), NUMBER_HINT),
  password: z.string().min(1, 'Enter your password'),
})

export type PhoneLinkOutcome = ConfirmOutcome | 'invalid-number'

interface PhoneLinkDialogProps {
  isOpen: boolean
  onOpenChange: (isOpen: boolean) => void
  /** Starts linking with the number's digits; the dialog closes once it is `confirmed`. */
  onConfirm: (digits: string, password: string) => Promise<PhoneLinkOutcome>
}

/** Takes the number of the phone to link, and the password the link is behind. */
export function PhoneLinkDialog({
  isOpen,
  onOpenChange,
  onConfirm,
}: PhoneLinkDialogProps) {
  return (
    <Dialog
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      purpose="form"
      width={440}
    >
      {/* Mounted only while open, so a typed password never survives a close. */}
      {isOpen ? (
        <PhoneLinkForm
          onConfirm={onConfirm}
          onClose={() => onOpenChange(false)}
        />
      ) : null}
    </Dialog>
  )
}

interface PhoneLinkFormProps {
  onConfirm: PhoneLinkDialogProps['onConfirm']
  onClose: () => void
}

function PhoneLinkForm({ onConfirm, onClose }: PhoneLinkFormProps) {
  const [problem, setProblem] = useState<ConfirmProblem | null>(null)

  const form = useForm({
    defaultValues: { phone_number: '', password: '' },
    validators: { onSubmit: schema },
    canSubmitWhenInvalid: true,
    listeners: {
      onChange: () => setProblem(null),
    },
    onSubmit: async ({ value, formApi }) => {
      let outcome: PhoneLinkOutcome
      try {
        outcome = await onConfirm(digitsOf(value.phone_number), value.password)
      } catch {
        outcome = 'failed'
      }
      if (outcome === 'confirmed') {
        onClose()
      } else if (outcome === 'incorrect-password') {
        formApi.setErrorMap({
          onSubmit: {
            fields: { password: [{ message: 'That password is incorrect' }] },
          },
        })
      } else if (outcome === 'invalid-number') {
        formApi.setErrorMap({
          onSubmit: { fields: { phone_number: [{ message: NUMBER_HINT }] } },
        })
      } else {
        setProblem(outcome)
      }
    },
  })

  return (
    <form
      noValidate
      onSubmit={(event) => {
        event.preventDefault()
        void form.handleSubmit()
      }}
    >
      <Layout
        header={
          <DialogHeader
            title="Link with a phone number"
            subtitle="Use the number of the phone with WhatsApp. You'll enter a code on it next."
            onOpenChange={() => onClose()}
          />
        }
        content={
          <LayoutContent>
            <VStack gap={4}>
              {problem ? <ConfirmProblemBanner problem={problem} /> : null}
              <FormLayout defaultOptionality="required">
                <form.Field name="phone_number">
                  {(field) => (
                    <PhoneNumberField
                      label="Phone number"
                      placeholder="+44 7700 900123"
                      htmlName={field.name}
                      value={field.state.value}
                      onChange={(value) => field.handleChange(value)}
                      onBlur={field.handleBlur}
                      isInvalid={
                        field.state.meta.isTouched && !field.state.meta.isValid
                      }
                      errorMessage={toStatusMessage(field.state.meta.errors)}
                    />
                  )}
                </form.Field>
                <form.Field name="password">
                  {(field) => (
                    <PasswordField
                      label="Password"
                      htmlName={field.name}
                      autoComplete="current-password"
                      value={field.state.value}
                      onChange={(value) => field.handleChange(value)}
                      onBlur={field.handleBlur}
                      isInvalid={
                        field.state.meta.isTouched && !field.state.meta.isValid
                      }
                      errorMessage={toStatusMessage(field.state.meta.errors)}
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
                    label="Continue"
                    variant="primary"
                    isLoading={isSubmitting}
                  />
                )}
              </form.Subscribe>
            </HStack>
          </LayoutFooter>
        }
      />
    </form>
  )
}

function digitsOf(value: string) {
  return value.replace(/\D/g, '')
}
