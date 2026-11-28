import { useState } from 'react'
import { useForm } from '@tanstack/react-form'
import { Button } from '@astryxdesign/core/Button'
import { Dialog, DialogHeader } from '@astryxdesign/core/Dialog'
import { FormLayout } from '@astryxdesign/core/FormLayout'
import { HStack } from '@astryxdesign/core/HStack'
import { Layout, LayoutContent, LayoutFooter } from '@astryxdesign/core/Layout'
import { VStack } from '@astryxdesign/core/VStack'
import { z } from 'zod'

import {
  ConfirmProblemBanner,
  PasskeyReauthButton,
} from '@/components/confirm-password-dialog'
import type {
  ConfirmOutcome,
  ConfirmProblem,
  Reauth,
} from '@/components/confirm-password-dialog'
import { PasswordField } from '@/components/password-field'
import { PhoneNumberField } from '@/components/phone-number-field'
import { toStatusMessage } from '@/lib/form-errors'
import { usePasskeyReauth } from '@/lib/use-passkey-reauth'
import type { PasskeyAnswer } from '@/lib/use-passkey-reauth'
import { NUMBER_HINT, digitsOf, isPhoneNumber } from '@/lib/whatsapp'

const schema = z.object({
  phone_number: z.string().trim().refine(isPhoneNumber, NUMBER_HINT),
  password: z.string().min(1, 'Enter your password'),
})

export type PhoneLinkOutcome = ConfirmOutcome | 'invalid-number'

interface PhoneLinkDialogProps {
  isOpen: boolean
  onOpenChange: (isOpen: boolean) => void
  /** Starts linking with the number's digits; the dialog closes once it is `confirmed`. */
  onConfirm: (digits: string, reauth: Reauth) => Promise<PhoneLinkOutcome>
}

/** Takes the number of the phone to link, and the password or passkey the link is behind. */
export function PhoneLinkDialog({
  isOpen,
  onOpenChange,
  onConfirm,
}: PhoneLinkDialogProps) {
  const passkey = usePasskeyReauth(isOpen)
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
          passkey={passkey}
          onClose={() => onOpenChange(false)}
        />
      ) : null}
    </Dialog>
  )
}

interface PhoneLinkFormProps {
  onConfirm: PhoneLinkDialogProps['onConfirm']
  passkey: ReturnType<typeof usePasskeyReauth>
  onClose: () => void
}

function PhoneLinkForm({ onConfirm, passkey, onClose }: PhoneLinkFormProps) {
  const [problem, setProblem] = useState<ConfirmProblem | null>(null)

  const confirm = async (phoneNumber: string, reauth: Reauth) => {
    try {
      return await onConfirm(digitsOf(phoneNumber), reauth)
    } catch {
      return 'failed' as const
    }
  }

  const form = useForm({
    defaultValues: { phone_number: '', password: '' },
    validators: { onSubmit: schema },
    canSubmitWhenInvalid: true,
    listeners: {
      onChange: () => setProblem(null),
    },
    onSubmit: async ({ value, formApi }) => {
      const outcome = await confirm(value.phone_number, {
        password: value.password,
      })
      if (outcome === 'confirmed') {
        onClose()
      } else if (outcome === 'incorrect-password') {
        formApi.setErrorMap({
          onSubmit: {
            fields: { password: [{ message: 'That password is incorrect' }] },
          },
        })
      } else if (outcome === 'invalid-number') {
        showNumberHint()
      } else {
        setProblem(outcome)
      }
    },
  })

  function showNumberHint() {
    form.setFieldMeta('phone_number', (meta) => ({ ...meta, isTouched: true }))
    form.setErrorMap({
      onSubmit: { fields: { phone_number: [{ message: NUMBER_HINT }] } },
    })
  }

  // Checked before the browser's prompt, which a bad number would only waste.
  const hasNumber = () => {
    if (isPhoneNumber(form.getFieldValue('phone_number').trim())) return true
    showNumberHint()
    return false
  }

  const confirmWithPasskey = async (answer: PasskeyAnswer) => {
    setProblem(null)
    if (answer === 'cancelled') return
    if (answer === 'failed') {
      setProblem('passkey-failed')
      return
    }
    const outcome = await confirm(form.getFieldValue('phone_number'), answer)
    if (outcome === 'confirmed') onClose()
    else if (outcome === 'invalid-number') showNumberHint()
    else if (outcome === 'incorrect-password') setProblem('passkey-rejected')
    else setProblem(outcome)
  }

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
              {passkey.isAvailable ? (
                <PasskeyReauthButton
                  canAnswer={hasNumber}
                  answer={passkey.answer}
                  onAnswer={confirmWithPasskey}
                />
              ) : null}
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
