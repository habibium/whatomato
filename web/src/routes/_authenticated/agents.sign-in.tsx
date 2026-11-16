import { useEffect, useMemo, useRef, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useForm, useStore } from '@tanstack/react-form'
import { createFileRoute, useLocation } from '@tanstack/react-router'
import { Banner } from '@astryxdesign/core/Banner'
import { Button } from '@astryxdesign/core/Button'
import { Divider } from '@astryxdesign/core/Divider'
import { HStack } from '@astryxdesign/core/HStack'
import { List, ListItem } from '@astryxdesign/core/List'
import { RadioList, RadioListItem } from '@astryxdesign/core/RadioList'
import { Skeleton } from '@astryxdesign/core/Skeleton'
import { Text } from '@astryxdesign/core/Text'
import { TextInput } from '@astryxdesign/core/TextInput'
import { VStack } from '@astryxdesign/core/VStack'
import { z } from 'zod'

import type { components } from '@/api/schema'

import { api } from '@/api/client'
import { PermissionEditor } from '@/components/agent-permissions'
import { AppFrame } from '@/components/app-frame'
import { ConfirmPasswordDialog } from '@/components/confirm-password-dialog'
import type { ConfirmOutcome } from '@/components/confirm-password-dialog'
import { ListEmpty } from '@/components/list-empty'
import { PageHeading } from '@/components/page-heading'
import { SettingsSection } from '@/components/settings-section'
import { cleanGrant } from '@/lib/agent-grant'
import type { Grant } from '@/lib/agent-grant'
import { signInCheckQuery, unpackRequest } from '@/lib/agent-sign-in'
import type { AuthorizationRequest, SignInAsk } from '@/lib/agent-sign-in'
import { agentsQuery, presetsQuery } from '@/lib/agents'
import type { AgentPreset } from '@/lib/agents'
import { toFieldErrors, toFieldStatus } from '@/lib/form-errors'
import { retryAfterSeconds } from '@/lib/retry-after'
import { stringParam } from '@/lib/search'
import { useSessionExpiry } from '@/lib/use-session-expiry'
import { ApiError } from '@/lib/whatsapp'
import { connectionProblem, toWriteProblem } from '@/lib/write-problem'
import type { WriteProblem } from '@/lib/write-problem'

export const Route = createFileRoute('/_authenticated/agents/sign-in')({
  // A plain function, not zod: route options load with every page
  validateSearch: (search): { request?: string } => ({
    request: stringParam(search.request),
  }),
  component: SignInPage,
})

const NEW = 'new'
const DEFAULT_PRESET = 'ask_before_sending'
const NAME_MAX = 60

const schema = z
  .object({
    choice: z.string(),
    name: z.string(),
    grant: z.custom<Grant>(),
  })
  .superRefine((values, context) => {
    if (values.choice !== NEW) return
    const name = values.name.trim()
    if (name.length < 1 || name.length > NAME_MAX) {
      context.addIssue({
        code: 'custom',
        path: ['name'],
        message: name
          ? `Keep it to ${NAME_MAX} characters`
          : 'Name the connection, such as “Claude on my phone”',
      })
    }
  })

type Values = z.infer<typeof schema>

function SignInPage() {
  const { request: packed } = Route.useSearch()
  const returnTo = useLocation({ select: (location) => location.href })
  const request = useMemo(() => unpackRequest(packed), [packed])

  return (
    <AppFrame>
      {request ? (
        <Asked request={request} returnTo={returnTo} />
      ) : (
        <ListEmpty
          title="Nothing is asking to connect"
          description="This page opens when an app such as Claude or ChatGPT asks to connect to your account. Start from the app, and it brings you here."
          actions={<Button label="Go to agents" href="/agents" />}
        />
      )}
    </AppFrame>
  )
}

function Asked({
  request,
  returnTo,
}: {
  request: AuthorizationRequest
  returnTo: string
}) {
  const check = useQuery(signInCheckQuery(request))
  const isOff = check.error instanceof ApiError && check.error.status === 404
  const presets = useQuery({ ...presetsQuery, enabled: !isOff })
  const isSignedOut = useSessionExpiry(check.error ?? presets.error, returnTo)
  const answer = check.data

  useEffect(() => {
    if (answer?.status === 'redirect') window.location.replace(answer.url)
  }, [answer])

  if (isOff) {
    return (
      <ListEmpty
        title="Agent access is off on this server"
        description="Whoever runs this Whatomato has turned off connecting AI agents."
        actions={<Button label="Go to messages" href="/messages" />}
      />
    )
  }
  if (answer?.status === 'refused') {
    return (
      <ListEmpty
        title="This app can’t be connected"
        description={answer.message}
        actions={<Button label="Go to agents" href="/agents" />}
      />
    )
  }
  if (answer?.status === 'redirect') return <Leaving />
  if (answer && presets.data) {
    return <Consent ask={answer} presets={presets.data} request={request} />
  }
  if (check.isPending || presets.isPending) {
    return (
      <VStack gap={4} aria-busy="true">
        {[0, 1, 2].map((index) => (
          <Skeleton key={index} index={index} width="100%" height={160} />
        ))}
      </VStack>
    )
  }
  if (isSignedOut) return null
  return (
    <Banner
      status="error"
      title="Couldn't check what is asking to connect."
      description="Check your connection and try again."
      endContent={
        <Button
          label="Try again"
          size="sm"
          clickAction={async () => {
            await Promise.all([check.refetch(), presets.refetch()])
          }}
        />
      }
    />
  )
}

function Leaving() {
  return (
    <VStack gap={2} aria-live="polite">
      <PageHeading
        title="Sending you back"
        lede="You can close this tab once the app shows it’s connected."
      />
    </VStack>
  )
}

function startingGrant(presets: readonly AgentPreset[]) {
  const preset =
    presets.find((item) => item.id === DEFAULT_PRESET) ?? presets.at(0)
  return preset ? cleanGrant(preset.grant) : null
}

interface ConsentProps {
  ask: SignInAsk
  presets: AgentPreset[]
  request: AuthorizationRequest
}

function Consent({ ask, presets, request }: ConsentProps) {
  const queryClient = useQueryClient()
  const formRef = useRef<HTMLFormElement>(null)
  const bannerRef = useRef<HTMLDivElement>(null)
  const [problem, setProblem] = useState<WriteProblem | null>(null)
  const [grantProblems, setGrantProblems] = useState<string[]>([])
  const [pending, setPending] = useState<Values | null>(null)
  const [isLeaving, setIsLeaving] = useState(false)
  const failed = `Couldn't connect ${ask.client_name}.`
  const canChoose = ask.connections.length > 0
  const isFull = !ask.can_add && !canChoose

  const form = useForm({
    defaultValues: {
      choice: ask.connections.at(0)?.id ?? NEW,
      name: ask.client_name.slice(0, NAME_MAX),
      grant: startingGrant(presets) as Grant,
    } as Values,
    validators: { onSubmit: schema },
    canSubmitWhenInvalid: true,
    listeners: {
      onChange: () => setProblem(null),
    },
    onSubmitInvalid: () => revealInvalid(formRef.current),
    onSubmit: ({ value }) => setPending(value),
  })
  const choice = useStore(form.store, (state) => state.values.choice)

  useEffect(() => {
    if (problem || grantProblems.length) {
      bannerRef.current?.scrollIntoView({ block: 'nearest' })
    }
  }, [problem, grantProblems])

  const approve = async (password: string): Promise<ConfirmOutcome> => {
    if (!pending) return 'failed'
    const body: components['schemas']['ApproveSignIn'] =
      pending.choice === NEW
        ? {
            request,
            password,
            name: pending.name.trim(),
            grant: cleanGrant(pending.grant),
          }
        : { request, password, connection_id: pending.choice }
    let result
    try {
      result = await api.POST('/api/agents/sign-in/approve', { body })
    } catch {
      setProblem(connectionProblem(failed))
      return 'confirmed'
    }
    if (result.data) {
      void queryClient.invalidateQueries({ queryKey: agentsQuery.queryKey })
      setIsLeaving(true)
      window.location.assign(result.data.url)
      return 'confirmed'
    }
    const { response, error } = result
    if (response.status === 429) {
      return { kind: 'rate-limited', retryAfter: retryAfterSeconds(response) }
    }
    if (toFieldErrors(error, ['password'] as const)) return 'incorrect-password'
    if (response.status === 409) {
      setProblem({
        title: 'You have 20 connections already.',
        description:
          'That’s the most an account can have. Revoke one you no longer use, then connect this app again.',
      })
      return 'confirmed'
    }
    if (response.status === 404) {
      setProblem({
        title: failed,
        description:
          'The connection you chose no longer exists. Reload this page to see what’s there now.',
      })
      return 'confirmed'
    }
    const details = (error as components['schemas']['ErrorBody'] | undefined)
      ?.details
    if (details?.request?.length) {
      setProblem({ title: failed, description: details.request.join(' ') })
      return 'confirmed'
    }
    const grant = details?.grant ?? []
    const fields = toFieldErrors(error, ['name'] as const)
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

  if (isLeaving) return <Leaving />

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
          <PageHeading
            title={`Connect ${ask.client_name}?`}
            lede="It’s asking to use your WhatsApp through Whatomato. It gets only what you allow here, and you can pause or revoke it whenever you like."
          />

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
            id="sign-in-app"
            title="Who is asking"
            description="What the app says about itself. Whatomato can check where its details are published, not that the name is true."
          >
            <List hasDividers density="compact">
              <ListItem
                label="Calls itself"
                endContent={<Text>{ask.client_name}</Text>}
              />
              <ListItem
                label="Published at"
                endContent={<Text>{ask.client_host}</Text>}
              />
              <ListItem
                label="Sends you back to"
                endContent={<Text>{ask.redirect_host}</Text>}
              />
            </List>
            {ask.is_local_only ? (
              <Banner
                status="warning"
                title="This app runs on this computer."
                description={`Any program on this computer can claim to be ${ask.client_name}. Go on only if you started this yourself, just now.`}
              />
            ) : null}
          </SettingsSection>

          <Divider />

          {isFull ? (
            <Banner
              status="error"
              title="You have 20 connections already."
              description="That’s the most an account can have. Revoke one you no longer use, then start again from the app."
              endContent={
                <Button label="Go to agents" size="sm" href="/agents" />
              }
            />
          ) : null}

          {canChoose ? (
            <>
              <SettingsSection
                id="sign-in-connection"
                title="Connection"
                description="This app has connected before. Signing in again keeps that connection’s permissions and its activity."
              >
                <form.Field name="choice">
                  {(field) => (
                    <RadioList
                      label="Connection"
                      isLabelHidden
                      value={field.state.value}
                      onChange={(value) => field.handleChange(value)}
                    >
                      {ask.connections.map((connection) => (
                        <RadioListItem
                          key={connection.id}
                          value={connection.id}
                          label={`Sign in again to ${connection.name}`}
                          description={
                            connection.grant_problem ??
                            connection.summary.slice(0, 2).join(' ')
                          }
                        />
                      ))}
                      {ask.can_add ? (
                        <RadioListItem
                          value={NEW}
                          label="Make a new connection"
                          description="With a name and permissions of its own."
                        />
                      ) : null}
                    </RadioList>
                  )}
                </form.Field>
              </SettingsSection>
              <Divider />
            </>
          ) : null}

          {choice === NEW && !isFull ? (
            <>
              <SettingsSection
                id="sign-in-name"
                title="Name"
                description="What to call this connection, so you can tell your agents apart."
              >
                <form.Field name="name">
                  {(field) => (
                    <TextInput
                      label="Name"
                      isLabelHidden
                      size="lg"
                      width="100%"
                      htmlName={field.name}
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
            </>
          ) : null}

          <HStack gap={2} justify="end">
            <Button
              label="Don’t allow"
              variant="secondary"
              clickAction={() => {
                setIsLeaving(true)
                window.location.assign(ask.deny_url)
              }}
            />
            {isFull ? null : (
              <Button
                type="submit"
                label={choice === NEW ? 'Allow' : 'Sign in again'}
                variant="primary"
              />
            )}
          </HStack>
        </VStack>
      </form>

      <ConfirmPasswordDialog
        isOpen={pending !== null}
        onOpenChange={(isOpen) => {
          if (!isOpen) setPending(null)
        }}
        title="Confirm it’s you"
        description={`${ask.client_name} will be able to act on your WhatsApp within these permissions, so connecting it needs your password.`}
        confirmLabel={choice === NEW ? 'Allow' : 'Sign in again'}
        onConfirm={approve}
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
