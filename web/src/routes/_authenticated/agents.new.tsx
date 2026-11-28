import { useEffect, useRef, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useForm, useStore } from '@tanstack/react-form'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { Banner } from '@astryxdesign/core/Banner'
import { Button } from '@astryxdesign/core/Button'
import { Divider } from '@astryxdesign/core/Divider'
import { Field } from '@astryxdesign/core/Field'
import { HStack } from '@astryxdesign/core/HStack'
import { NumberInput } from '@astryxdesign/core/NumberInput'
import {
  SegmentedControl,
  SegmentedControlItem,
} from '@astryxdesign/core/SegmentedControl'
import { Skeleton } from '@astryxdesign/core/Skeleton'
import { TextInput } from '@astryxdesign/core/TextInput'
import { VStack } from '@astryxdesign/core/VStack'
import { z } from 'zod'

import type { components } from '@/api/schema'

import { api } from '@/api/client'
import { KeyIssued } from '@/components/agent-key-issued'
import { PermissionEditor } from '@/components/agent-permissions'
import { AppFrame } from '@/components/app-frame'
import {
  ConfirmPasswordDialog,
  toReauthProblem,
} from '@/components/confirm-password-dialog'
import type {
  ConfirmOutcome,
  Reauth,
} from '@/components/confirm-password-dialog'
import { ListEmpty } from '@/components/list-empty'
import { PageHeading } from '@/components/page-heading'
import { SettingsSection } from '@/components/settings-section'
import { cleanGrant } from '@/lib/agent-grant'
import type { Grant } from '@/lib/agent-grant'
import { agentsQuery, presetsQuery } from '@/lib/agents'
import type { AgentKeyIssued, AgentPreset } from '@/lib/agents'
import { toFieldErrors, toFieldStatus } from '@/lib/form-errors'
import { retryAfterSeconds } from '@/lib/retry-after'
import { useSessionExpiry } from '@/lib/use-session-expiry'
import { ApiError } from '@/lib/whatsapp'
import { connectionProblem, toWriteProblem } from '@/lib/write-problem'
import type { WriteProblem } from '@/lib/write-problem'

export const Route = createFileRoute('/_authenticated/agents/new')({
  component: NewConnectionPage,
})

const DEFAULT_PRESET = 'ask_before_sending'
const NAME_MAX = 60
const DAYS_MAX = 365

const schema = z
  .object({
    name: z
      .string()
      .trim()
      .min(1, 'Name the connection, such as “Claude Code on my laptop”')
      .max(NAME_MAX, `Keep it to ${NAME_MAX} characters`),
    grant: z.custom<Grant>(),
    expiry: z.enum(['never', 'days']),
    expires_in_days: z.number().nullable(),
  })
  .superRefine((values, context) => {
    const days = values.expires_in_days
    if (
      values.expiry === 'days' &&
      (days === null || !Number.isInteger(days) || days < 1 || days > DAYS_MAX)
    ) {
      context.addIssue({
        code: 'custom',
        path: ['expires_in_days'],
        message: `Enter a number of days from 1 to ${DAYS_MAX}`,
      })
    }
  })

type Values = z.infer<typeof schema>

function NewConnectionPage() {
  const navigate = useNavigate()
  const overview = useQuery(agentsQuery)
  const isEnabled = overview.data?.enabled
  const presets = useQuery({ ...presetsQuery, enabled: isEnabled === true })
  const isSignedOut = useSessionExpiry(
    overview.error ?? presets.error,
    '/agents/new',
  )
  // Only ever here: never in the URL, storage or the query cache
  const [issued, setIssued] = useState<AgentKeyIssued | null>(null)

  if (issued) {
    return (
      <AppFrame>
        <KeyIssued
          issued={issued}
          title={`${issued.connection.name} is ready`}
          lede="Give the agent its access key, then follow the steps for it below."
          onDone={() => navigate({ to: '/agents' })}
        />
      </AppFrame>
    )
  }

  const isOff =
    isEnabled === false ||
    (presets.error instanceof ApiError && presets.error.status === 404)

  return (
    <AppFrame>
      <VStack gap={8}>
        <PageHeading
          title="New connection"
          lede="An access key for one AI agent, and what it may do with your account."
        />
        {isOff ? (
          <ListEmpty
            title="Agent access is off on this server"
            description="Whoever runs this Whatomato has turned off connecting AI agents."
            actions={<Button label="Go to messages" href="/messages" />}
          />
        ) : presets.data ? (
          <CreateForm presets={presets.data} onCreated={setIssued} />
        ) : overview.isPending || (isEnabled === true && presets.isPending) ? (
          <VStack gap={4} aria-busy="true">
            {[0, 1, 2].map((index) => (
              <Skeleton key={index} index={index} width="100%" height={160} />
            ))}
          </VStack>
        ) : isSignedOut ? null : (
          <Banner
            status="error"
            title="Couldn't load the permission choices."
            description="Check your connection and try again."
            endContent={
              <Button
                label="Try again"
                size="sm"
                clickAction={async () => {
                  await Promise.all([overview.refetch(), presets.refetch()])
                }}
              />
            }
          />
        )}
      </VStack>
    </AppFrame>
  )
}

function startingGrant(presets: readonly AgentPreset[]) {
  const preset =
    presets.find((item) => item.id === DEFAULT_PRESET) ?? presets.at(0)
  return preset ? cleanGrant(preset.grant) : null
}

interface CreateFormProps {
  presets: AgentPreset[]
  onCreated: (issued: AgentKeyIssued) => void
}

function CreateForm({ presets, onCreated }: CreateFormProps) {
  const queryClient = useQueryClient()
  const formRef = useRef<HTMLFormElement>(null)
  const bannerRef = useRef<HTMLDivElement>(null)
  const [problem, setProblem] = useState<WriteProblem | null>(null)
  const [grantProblems, setGrantProblems] = useState<string[]>([])
  const [pending, setPending] = useState<Values | null>(null)
  const failed = "Couldn't create the connection."

  const form = useForm({
    defaultValues: {
      name: '',
      grant: startingGrant(presets) as Grant,
      expiry: 'never',
      expires_in_days: 30,
    } as Values,
    validators: { onSubmit: schema },
    canSubmitWhenInvalid: true,
    listeners: {
      onChange: () => setProblem(null),
    },
    onSubmitInvalid: () => revealInvalid(formRef.current),
    onSubmit: ({ value }) => setPending(value),
  })

  useEffect(() => {
    if (problem || grantProblems.length) {
      bannerRef.current?.scrollIntoView({ block: 'nearest' })
    }
  }, [problem, grantProblems])

  const create = async (reauth: Reauth): Promise<ConfirmOutcome> => {
    if (!pending) return 'failed'
    const body: components['schemas']['CreateConnection'] = {
      name: pending.name.trim(),
      grant: cleanGrant(pending.grant),
      expires_in_days:
        pending.expiry === 'days' ? pending.expires_in_days : null,
      ...reauth,
    }
    let result
    try {
      result = await api.POST('/api/agents', { body })
    } catch {
      setProblem(connectionProblem(failed))
      return 'confirmed'
    }
    if (result.data) {
      await queryClient.invalidateQueries({ queryKey: agentsQuery.queryKey })
      onCreated(result.data)
      return 'confirmed'
    }
    const { response, error } = result
    if (response.status === 429) {
      return { kind: 'rate-limited', retryAfter: retryAfterSeconds(response) }
    }
    const reauthProblem = toReauthProblem(error)
    if (reauthProblem) return reauthProblem
    if (response.status === 409) {
      setProblem({
        title: 'You have 20 connections already.',
        description:
          'That’s the most an account can have. Revoke one you no longer use, then create this one.',
      })
      return 'confirmed'
    }
    const details = (error as components['schemas']['ErrorBody'] | undefined)
      ?.details
    const grant = details?.grant ?? []
    const fields = toFieldErrors(error, ['name', 'expires_in_days'] as const)
    if (grant.length || fields) {
      setGrantProblems(grant)
      if (fields) {
        form.setErrorMap({ onSubmit: { fields } })
        revealInvalid(formRef.current)
      }
      return 'confirmed'
    }
    setProblem(toWriteProblem(result, failed))
    return 'confirmed'
  }

  const expiry = useStore(form.store, (state) => state.values.expiry)

  return (
    <>
      <form
        ref={formRef}
        noValidate
        onSubmit={(event) => {
          event.preventDefault()
          void form.handleSubmit()
        }}
      >
        <VStack gap={8}>
          {problem ? (
            <VStack ref={bannerRef}>
              <Banner
                status="error"
                title={problem.title}
                description={problem.description}
              />
            </VStack>
          ) : null}

          <SettingsSection
            id="connection-name"
            title="Name"
            description="What the agent is, so you can tell your connections apart."
          >
            <form.Field name="name">
              {(field) => (
                <TextInput
                  label="Name"
                  isLabelHidden
                  size="lg"
                  width="100%"
                  htmlName={field.name}
                  placeholder="Claude Code on my laptop"
                  autoComplete="off"
                  value={field.state.value}
                  onChange={(value) => field.handleChange(value)}
                  onBlur={field.handleBlur}
                  status={toFieldStatus(field.state.meta)}
                />
              )}
            </form.Field>
          </SettingsSection>

          <Divider />

          <form.Field name="grant">
            {(field) => (
              <VStack ref={grantProblems.length ? bannerRef : undefined}>
                <PermissionEditor
                  value={field.state.value}
                  onChange={(grant) => {
                    setGrantProblems([])
                    field.handleChange(grant)
                  }}
                  presets={presets}
                  problems={grantProblems}
                />
              </VStack>
            )}
          </form.Field>

          <Divider />

          <SettingsSection
            id="connection-expiry"
            title="Expiry"
            description="When the access key stops working. You can pause or revoke it sooner."
          >
            <form.Field name="expiry">
              {(field) => (
                <Field
                  label="Expires"
                  inputID="expiry"
                  isGroupLabel
                  isLabelHidden
                >
                  <HStack>
                    <SegmentedControl
                      label="Expires"
                      value={field.state.value}
                      onChange={(value) =>
                        field.handleChange(value as Values['expiry'])
                      }
                    >
                      <SegmentedControlItem value="never" label="Never" />
                      <SegmentedControlItem
                        value="days"
                        label="After some days"
                      />
                    </SegmentedControl>
                  </HStack>
                </Field>
              )}
            </form.Field>
            {expiry === 'days' ? (
              <form.Field name="expires_in_days">
                {(field) => (
                  <NumberInput
                    label="Days until it expires"
                    width={240}
                    units="days"
                    min={1}
                    max={DAYS_MAX}
                    isIntegerOnly
                    isWheelEnabled={false}
                    value={field.state.value}
                    onChange={(value: number) => field.handleChange(value)}
                    status={toFieldStatus(field.state.meta)}
                  />
                )}
              </form.Field>
            ) : null}
          </SettingsSection>

          <Divider />

          <HStack gap={2} justify="end">
            <Button label="Cancel" variant="secondary" href="/agents" />
            <Button type="submit" label="Create connection" variant="primary" />
          </HStack>
        </VStack>
      </form>

      <ConfirmPasswordDialog
        isOpen={pending !== null}
        onOpenChange={(isOpen) => {
          if (!isOpen) setPending(null)
        }}
        title="Confirm it’s you"
        description="An access key can send messages from your number, so creating one needs your password or a passkey."
        confirmLabel="Create connection"
        onConfirm={create}
      />
    </>
  )
}

/** A long form can fail on a field that's out of view. */
function revealInvalid(form: HTMLFormElement | null) {
  requestAnimationFrame(() => {
    form
      ?.querySelector('[aria-invalid="true"]')
      ?.scrollIntoView({ block: 'center' })
  })
}
