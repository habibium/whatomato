import { useState } from 'react'
import { AlertDialog } from '@astryxdesign/core/AlertDialog'
import { Banner } from '@astryxdesign/core/Banner'
import { Button } from '@astryxdesign/core/Button'
import { HStack } from '@astryxdesign/core/HStack'
import { Icon } from '@astryxdesign/core/Icon'
import { List, ListItem } from '@astryxdesign/core/List'
import { Text } from '@astryxdesign/core/Text'
import { Timestamp } from '@astryxdesign/core/Timestamp'
import { KeyRound, Plus } from 'lucide-react'

import { $api, api } from '@/api/client'
import {
  ConfirmPasswordDialog,
  toConfirmOutcome,
} from '@/components/confirm-password-dialog'
import type {
  ConfirmOutcome,
  Reauth,
} from '@/components/confirm-password-dialog'
import { PasskeyNameDialog } from '@/components/passkey-name-dialog'
import { SettingsSection } from '@/components/settings-section'
import type { Passkey, SaveOutcome } from '@/components/passkey-name-dialog'
import {
  createPasskey,
  hasPasskeySupport,
  toPasskeyFailure,
} from '@/lib/passkey'
import type { WebAuthnJSON } from '@/lib/passkey'

type Notice =
  | Exclude<SaveOutcome, 'saved'>
  | 'added'
  | 'removed'
  | 'device-registered'
  | 'limit'

/** Lists the user's passkeys and adds or removes them, each change behind the password or a passkey. */
export function PasskeysSection() {
  const [isSupported] = useState(hasPasskeySupport)
  const passkeys = $api.useQuery('get', '/api/auth/passkeys')
  const [notice, setNotice] = useState<Notice | null>(null)
  const [isAdding, setIsAdding] = useState(false)
  const [created, setCreated] = useState<WebAuthnJSON | null>(null)
  const [readyToCreate, setReadyToCreate] = useState<WebAuthnJSON | null>(null)
  const [removing, setRemoving] = useState<Passkey | null>(null)

  const startAdding = () => {
    setNotice(null)
    setIsAdding(true)
  }

  const create = async (publicKey: WebAuthnJSON) => {
    try {
      setCreated(await createPasskey(publicKey))
    } catch (error) {
      const failure = toPasskeyFailure(error)
      if (failure === 'already-registered') setNotice('device-registered')
      else if (failure === 'failed') setNotice('failed')
    }
  }

  // Runs inside the password's submit, so the browser's prompt still follows a click. A passkey's
  // own prompt has used that click up, and Safari wants another before the next one.
  const register = async (reauth: Reauth): Promise<ConfirmOutcome> => {
    const result = await api.POST('/api/auth/passkeys/registration/start', {
      body: reauth,
    })
    if (!result.data) {
      if (result.response.status !== 409) return toConfirmOutcome(result)
      setNotice('limit')
      return 'confirmed'
    }
    if ('passkey' in reauth) setReadyToCreate(result.data.publicKey)
    else await create(result.data.publicKey)
    return 'confirmed'
  }

  const finishAdding = (outcome: SaveOutcome) => {
    setCreated(null)
    setNotice(outcome === 'saved' ? 'added' : outcome)
    void passkeys.refetch()
  }

  const remove = async (reauth: Reauth): Promise<ConfirmOutcome> => {
    if (!removing) return 'confirmed'
    const result = await api.DELETE('/api/auth/passkeys/{id}', {
      params: { path: { id: removing.id } },
      body: reauth,
    })
    // Already gone counts as removed: the list catches up either way.
    const outcome =
      result.response.status === 404 ? 'confirmed' : toConfirmOutcome(result)
    if (outcome === 'confirmed') {
      setNotice('removed')
      void passkeys.refetch()
    }
    return outcome
  }

  const list = passkeys.data ?? []
  if (!isSupported && list.length === 0) return null

  const addButton = isSupported ? (
    <Button
      label="Add a passkey"
      icon={<Icon icon={Plus} size="sm" />}
      clickAction={startAdding}
    />
  ) : null

  return (
    <>
      <SettingsSection
        id="passkeys"
        title="Passkeys"
        description="Sign in with your fingerprint, face or screen lock instead of your password. A passkey only works on this site, so it can't be phished."
      >
        {notice ? <NoticeBanner key={notice} notice={notice} /> : null}

        {passkeys.isError ? (
          <Banner
            status="error"
            title="Couldn't load your passkeys."
            description="Something went wrong on our side."
            endContent={
              <Button
                label="Try again"
                size="sm"
                clickAction={async () => {
                  await passkeys.refetch()
                }}
              />
            }
          />
        ) : list.length > 0 ? (
          <List hasDividers>
            {list.map((passkey) => (
              <ListItem
                key={passkey.id}
                label={passkey.name}
                startContent={<Icon icon={KeyRound} color="secondary" />}
                description={<PasskeyDates passkey={passkey} />}
                endContent={
                  <Button
                    label={`Remove ${passkey.name}`}
                    variant="ghost"
                    size="sm"
                    clickAction={() => setRemoving(passkey)}
                  >
                    Remove
                  </Button>
                }
              />
            ))}
          </List>
        ) : passkeys.isSuccess ? (
          <HStack gap={4} justify="between" vAlign="center" wrap="wrap">
            <Text color="secondary">
              No passkeys yet. Add one on this device, and next time sign in
              without typing your password.
            </Text>
            {addButton}
          </HStack>
        ) : null}
        {list.length > 0 && addButton ? <HStack>{addButton}</HStack> : null}
      </SettingsSection>

      <ConfirmPasswordDialog
        isOpen={isAdding}
        onOpenChange={setIsAdding}
        title="Add a passkey."
        description="Confirm it's you first. Your device then asks you to create the passkey."
        confirmLabel="Continue"
        onConfirm={register}
      />
      <AlertDialog
        isOpen={readyToCreate !== null}
        onOpenChange={(isOpen) => {
          if (!isOpen) setReadyToCreate(null)
        }}
        title="Create the passkey."
        description="Your device asks you to create it next."
        actionLabel="Create passkey"
        actionVariant="primary"
        onAction={() => {
          const publicKey = readyToCreate
          setReadyToCreate(null)
          if (publicKey) void create(publicKey)
        }}
      />
      <PasskeyNameDialog credential={created} onDone={finishAdding} />
      <ConfirmPasswordDialog
        isOpen={removing !== null}
        onOpenChange={(isOpen) => {
          if (!isOpen) setRemoving(null)
        }}
        title={`Remove ${removing?.name ?? 'passkey'}?`}
        description="It stops signing in to your account at once."
        confirmLabel="Remove passkey"
        onConfirm={remove}
      />
    </>
  )
}

function PasskeyDates({ passkey }: { passkey: Passkey }) {
  return (
    <Text type="supporting">
      Added{' '}
      <Timestamp value={passkey.created_at} format="date" type="inherit" />
      {' · '}
      {passkey.last_used_at ? (
        <>
          Last used{' '}
          <Timestamp
            value={passkey.last_used_at}
            format="auto"
            type="inherit"
          />
        </>
      ) : (
        'Not used yet'
      )}
    </Text>
  )
}

function NoticeBanner({ notice }: { notice: Notice }) {
  switch (notice) {
    case 'added':
      return (
        <Banner
          status="success"
          title="Passkey added."
          description="Next time, sign in with it instead of your password."
          isDismissable
        />
      )
    case 'removed':
      return (
        <Banner
          status="success"
          title="Passkey removed."
          description="It can no longer sign in to your account."
          isDismissable
        />
      )
    case 'device-registered':
      return (
        <Banner
          status="warning"
          title="This device already has a passkey here."
          description="It's in your list. To replace it, remove it first."
          isDismissable
        />
      )
    case 'limit':
      return (
        <Banner
          status="warning"
          title="That's as many passkeys as an account can hold."
          description="Remove one you no longer use to add another."
          isDismissable
        />
      )
    case 'expired':
      return (
        <Banner
          status="error"
          title="That took too long."
          description="Add the passkey again; your device will replace the one it just made."
          isDismissable
        />
      )
    case 'already-registered':
      return (
        <Banner
          status="error"
          title="That passkey is already registered."
          description="It may belong to another account. Try a different device."
          isDismissable
        />
      )
    case 'failed':
      return (
        <Banner
          status="error"
          title="Couldn't add the passkey."
          description="Something went wrong. Try again in a moment."
          isDismissable
        />
      )
  }
}
