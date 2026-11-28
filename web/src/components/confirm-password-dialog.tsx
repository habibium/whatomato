import { useState } from 'react'
import { useForm } from '@tanstack/react-form'
import { Banner } from '@astryxdesign/core/Banner'
import { Button } from '@astryxdesign/core/Button'
import { Dialog, DialogHeader } from '@astryxdesign/core/Dialog'
import { Divider } from '@astryxdesign/core/Divider'
import { FormLayout } from '@astryxdesign/core/FormLayout'
import { HStack } from '@astryxdesign/core/HStack'
import { Icon } from '@astryxdesign/core/Icon'
import { Layout, LayoutContent, LayoutFooter } from '@astryxdesign/core/Layout'
import { VStack } from '@astryxdesign/core/VStack'
import { KeyRound } from 'lucide-react'
import { z } from 'zod'

import { PasswordField } from '@/components/password-field'
import { toFieldErrors, toStatusMessage } from '@/lib/form-errors'
import {
  TOO_MANY_ATTEMPTS,
  retryAfterSeconds,
  tryAgainIn,
} from '@/lib/retry-after'
import { usePasskeyReauth } from '@/lib/use-passkey-reauth'
import type { PasskeyAnswer, Reauth } from '@/lib/use-passkey-reauth'

export type { Reauth } from '@/lib/use-passkey-reauth'

const schema = z.object({
  password: z.string().min(1, 'Enter your password'),
})

export type ConfirmOutcome =
  | 'confirmed'
  | 'incorrect-password'
  | 'passkey-rejected'
  | 'failed'
  | { kind: 'rate-limited'; retryAfter?: number }

export type ConfirmProblem =
  | Exclude<ConfirmOutcome, 'confirmed' | 'incorrect-password'>
  | 'passkey-failed'

/** Reads an openapi-fetch result from an endpoint that re-checks the password or a passkey. */
export function toConfirmOutcome(result: {
  response: Response
  error?: unknown
}): ConfirmOutcome {
  if (result.response.ok) return 'confirmed'
  if (result.response.status === 429) {
    return {
      kind: 'rate-limited',
      retryAfter: retryAfterSeconds(result.response),
    }
  }
  return toReauthProblem(result.error) ?? 'failed'
}

/** Whether a refusal was about the password or passkey; `null` for any other. */
export function toReauthProblem(error: unknown) {
  if (toFieldErrors(error, ['password'] as const)) {
    return 'incorrect-password' as const
  }
  if (toFieldErrors(error, ['passkey'] as const)) {
    return 'passkey-rejected' as const
  }
  return null
}

interface ConfirmPasswordDialogProps {
  isOpen: boolean
  onOpenChange: (isOpen: boolean) => void
  title: string
  description: string
  confirmLabel: string
  /** `destructive` for a change that can't be undone from here. */
  confirmVariant?: 'primary' | 'destructive'
  /**
   * Makes the protected change with the password or a passkey; the dialog closes once it is
   * `confirmed`.
   */
  onConfirm: (reauth: Reauth) => Promise<ConfirmOutcome>
}

/**
 * Asks for the password again, or a passkey where the account has one, before a change a stolen
 * session must not be able to make.
 */
export function ConfirmPasswordDialog({
  isOpen,
  onOpenChange,
  ...props
}: ConfirmPasswordDialogProps) {
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
        <ConfirmPasswordForm
          {...props}
          passkey={passkey}
          onClose={() => onOpenChange(false)}
        />
      ) : null}
    </Dialog>
  )
}

interface ConfirmPasswordFormProps extends Omit<
  ConfirmPasswordDialogProps,
  'isOpen' | 'onOpenChange'
> {
  passkey: ReturnType<typeof usePasskeyReauth>
  onClose: () => void
}

function ConfirmPasswordForm({
  title,
  description,
  confirmLabel,
  confirmVariant = 'primary',
  onConfirm,
  passkey,
  onClose,
}: ConfirmPasswordFormProps) {
  const [problem, setProblem] = useState<ConfirmProblem | null>(null)

  const confirm = async (reauth: Reauth) => {
    try {
      return await onConfirm(reauth)
    } catch {
      return 'failed' as const
    }
  }

  const form = useForm({
    defaultValues: { password: '' },
    validators: { onSubmit: schema },
    canSubmitWhenInvalid: true,
    listeners: {
      onChange: () => setProblem(null),
    },
    onSubmit: async ({ value, formApi }) => {
      const outcome = await confirm({ password: value.password })
      if (outcome === 'confirmed') {
        onClose()
      } else if (outcome === 'incorrect-password') {
        formApi.setErrorMap({
          onSubmit: {
            fields: { password: [{ message: 'That password is incorrect' }] },
          },
        })
      } else {
        setProblem(outcome)
      }
    },
  })

  const confirmWithPasskey = async (answer: PasskeyAnswer) => {
    setProblem(null)
    if (answer === 'cancelled') return
    if (answer === 'failed') {
      setProblem('passkey-failed')
      return
    }
    const outcome = await confirm(answer)
    if (outcome === 'confirmed') onClose()
    // A passkey answer leaves the password field alone, so its refusal is a banner too.
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
            title={title}
            subtitle={description}
            onOpenChange={() => onClose()}
          />
        }
        content={
          <LayoutContent>
            <VStack gap={4}>
              {problem ? <ConfirmProblemBanner problem={problem} /> : null}
              <FormLayout defaultOptionality="required">
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
                    label={confirmLabel}
                    variant={confirmVariant}
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

interface PasskeyReauthButtonProps {
  /** Checks the rest of the form first; `false` keeps the browser's prompt closed. */
  canAnswer?: () => boolean
  answer: () => Promise<PasskeyAnswer>
  /** Runs once the browser's prompt has closed, with what it gave. */
  onAnswer: (answer: PasskeyAnswer) => Promise<void>
}

/** "Use a passkey", under the password field of a form that asks for it again. */
export function PasskeyReauthButton({
  canAnswer,
  answer,
  onAnswer,
}: PasskeyReauthButtonProps) {
  const [isBusy, setIsBusy] = useState(false)
  return (
    <VStack gap={4}>
      <Divider label="or" />
      <Button
        label="Use a passkey"
        icon={<Icon icon={KeyRound} size="sm" />}
        width="100%"
        isLoading={isBusy}
        clickAction={async () => {
          if (canAnswer && !canAnswer()) return
          setIsBusy(true)
          try {
            await onAnswer(await answer())
          } finally {
            setIsBusy(false)
          }
        }}
      />
    </VStack>
  )
}

export function ConfirmProblemBanner({ problem }: { problem: ConfirmProblem }) {
  if (typeof problem === 'object') {
    return (
      <Banner
        status="error"
        title={TOO_MANY_ATTEMPTS}
        description={tryAgainIn(problem.retryAfter)}
      />
    )
  }
  switch (problem) {
    case 'passkey-rejected':
      return (
        <Banner
          status="error"
          title="That passkey wasn't accepted."
          description="It may have been removed from your account, or the check took too long. Try again, or use your password."
        />
      )
    case 'passkey-failed':
      return (
        <Banner
          status="error"
          title="Couldn't use a passkey."
          description="Something went wrong. Try again in a moment, or use your password."
        />
      )
    case 'failed':
      return (
        <Banner
          status="error"
          title="That didn't work."
          description="Something went wrong on our side. Try again in a moment."
        />
      )
  }
}
