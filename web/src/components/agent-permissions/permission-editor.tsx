import { useQuery } from '@tanstack/react-query'
import { useId, useMemo, useState } from 'react'
import { Banner } from '@astryxdesign/core/Banner'
import { Divider } from '@astryxdesign/core/Divider'
import { Field } from '@astryxdesign/core/Field'
import { HStack } from '@astryxdesign/core/HStack'
import { Heading, Text } from '@astryxdesign/core/Text'
import { RadioList, RadioListItem } from '@astryxdesign/core/RadioList'
import {
  SegmentedControl,
  SegmentedControlItem,
} from '@astryxdesign/core/SegmentedControl'
import { VStack } from '@astryxdesign/core/VStack'

import { ChatPicker } from './chat-picker'
import { ChatRuleList } from './chat-rule-list'
import {
  DailyLimitField,
  NoteField,
  ReadControl,
  SendControl,
} from './rule-fields'
import { SettingsSection } from '@/components/settings-section'
import {
  MAX_CHAT_RULES,
  SEND_LABELS,
  WARNINGS,
  addChatRule,
  newChatSend,
  chatKey,
  cleanGrant,
  grantWarning,
  matchingPreset,
  removeChatRule,
  updateChatRule,
  updateChats,
  updateDefaultRule,
} from '@/lib/agent-grant'
import type { Grant, Scope, Strangers } from '@/lib/agent-grant'
import { readingQuery } from '@/lib/agents'
import type { AgentPreset } from '@/lib/agents'

const CUSTOM = 'custom'

export interface PermissionEditorProps {
  value: Grant
  onChange: (grant: Grant) => void
  /** From `GET /api/agents/presets`, in display order. */
  presets: readonly AgentPreset[]
  /** The server's sentences on why it refused these permissions. */
  problems?: readonly string[]
}

/**
 * What an agent may do: a preset, the chats and their rules, people who aren't saved contacts, and
 * the rest of the account. Controlled; it knows nothing of where the permissions are saved.
 */
export function PermissionEditor({
  value,
  onChange,
  presets,
  problems,
}: PermissionEditorProps) {
  return (
    <VStack gap={8}>
      {problems?.length ? (
        <Banner
          status="error"
          title="These permissions can’t be saved."
          description={problems.join(' ')}
        />
      ) : null}
      <PresetSection value={value} onChange={onChange} presets={presets} />
      <Divider />
      <ChatsSection value={value} onChange={onChange} />
      <Divider />
      <StrangersSection value={value} onChange={onChange} />
      <Divider />
      <AccountSection value={value} onChange={onChange} />
    </VStack>
  )
}

type SectionProps = Pick<PermissionEditorProps, 'value' | 'onChange'>

function PresetSection({
  value,
  onChange,
  presets,
}: SectionProps & { presets: readonly AgentPreset[] }) {
  // Choosing Custom on a grant that equals a preset keeps Custom shown until a preset is chosen again
  const [isCustomChosen, setIsCustomChosen] = useState(false)
  const match = matchingPreset(value, presets)
  const chosen = isCustomChosen || !match ? CUSTOM : match.id
  const warning = grantWarning(value)

  return (
    <SettingsSection
      id="permissions-preset"
      title="What it may do"
      description="Start from one of these. Anything you change below makes it custom."
    >
      <RadioList
        label="What it may do"
        isLabelHidden
        value={chosen}
        onChange={(id) => {
          if (id === CUSTOM) {
            setIsCustomChosen(true)
            return
          }
          const preset = presets.find((item) => item.id === id)
          if (!preset) return
          setIsCustomChosen(false)
          onChange(cleanGrant(preset.grant))
        }}
        status={warning ?? undefined}
      >
        {presets.map((preset) => (
          <RadioListItem
            key={preset.id}
            value={preset.id}
            label={preset.name}
            description={preset.description}
          />
        ))}
        <RadioListItem
          value={CUSTOM}
          label="Custom"
          description="Set each permission yourself, below."
        />
      </RadioList>
    </SettingsSection>
  )
}

function ChatsSection({ value, onChange }: SectionProps) {
  const reading = useQuery(readingQuery)
  const scopeID = useId()
  const { scope, rules } = value.chats
  const listed = useMemo(
    () => new Set(rules.map((rule) => chatKey(rule.chat))),
    [rules],
  )
  const isFull = rules.length >= MAX_CHAT_RULES
  const isAll = scope === 'all'

  const list = (
    <VStack gap={3}>
      <ChatPicker
        listed={listed}
        onAdd={(chat) => onChange(addChatRule(value, chat))}
        isDisabled={isFull}
        disabledMessage={`A connection can list up to ${MAX_CHAT_RULES} chats.`}
      />
      {rules.length > 0 ? (
        <ChatRuleList
          hasReading={reading.data?.available === true}
          rules={rules}
          onChange={(key, change) =>
            onChange(updateChatRule(value, key, change))
          }
          onRemove={(key) => onChange(removeChatRule(value, key))}
        />
      ) : (
        <Text type="supporting">
          {isAll
            ? 'No exceptions. Every chat follows the rule above.'
            : 'No chats yet, so it can’t send to anyone. Add the chats it may use.'}
        </Text>
      )}
    </VStack>
  )

  return (
    <SettingsSection
      id="permissions-chats"
      title="Chats"
      description="Which chats it may reach, and whether it asks you before each message."
    >
      <Field label="Chats" inputID={scopeID} isGroupLabel isLabelHidden>
        <SegmentedControl
          label="Chats"
          layout="fill"
          value={scope}
          onChange={(next) =>
            onChange(updateChats(value, { scope: next as Scope }))
          }
        >
          <SegmentedControlItem value="all" label="All chats" />
          <SegmentedControlItem value="selected" label="Only selected chats" />
        </SegmentedControl>
      </Field>

      {reading.data?.available ? (
        <Text color="secondary">
          For a chat it may read, Whatomato keeps the last{' '}
          {reading.data.kept_days} days of messages, encrypted, so the agent can
          read and search them. No other chat is stored, reading shows nothing
          to the people in the chat, and what it kept is deleted within minutes
          of you turning reading off.
        </Text>
      ) : null}

      {isAll ? (
        <>
          {reading.data?.available ? (
            <ReadControl
              label="It may read messages"
              description="For every chat without its own rule."
              value={value.chats.default.read === true}
              warning={
                value.chats.default.read && value.chats.default.send === 'yes'
                  ? WARNINGS.readAndSend
                  : null
              }
              onChange={(read) => onChange(updateDefaultRule(value, { read }))}
            />
          ) : null}
          <SendControl
            label="May it send"
            description="For every chat without its own rule."
            value={value.chats.default.send}
            warning={
              value.chats.default.send === 'yes' ? WARNINGS.sendDefault : null
            }
            onChange={(send) => onChange(updateDefaultRule(value, { send }))}
          />
          <DailyLimitField
            label="Daily limit per chat"
            description="Messages a day to any one chat. Past it, the agent is refused."
            width={240}
            value={value.chats.default.daily_limit}
            isDisabled={value.chats.default.send === 'no'}
            onChange={(daily_limit) =>
              onChange(updateDefaultRule(value, { daily_limit }))
            }
          />
          <NoteField
            label="Note"
            value={value.chats.default.note}
            onChange={(note) => onChange(updateDefaultRule(value, { note }))}
          />
          <Divider />
          <VStack gap={1}>
            <Heading level={3}>Exceptions</Heading>
            <Text color="secondary">
              Chats with a rule of their own, in place of the one above. A chat
              you add starts as “
              {SEND_LABELS[newChatSend(value.chats.default.send)]}”.
            </Text>
          </VStack>
          {list}
        </>
      ) : (
        <>
          <Text color="secondary">
            It can reach only the chats listed here. A chat you add starts as “
            {SEND_LABELS[newChatSend(value.chats.default.send)]}”.
          </Text>
          {list}
        </>
      )}
    </SettingsSection>
  )
}

function StrangersSection({ value, onChange }: SectionProps) {
  const id = useId()
  const strangers = value.chats.strangers
  return (
    <SettingsSection
      id="permissions-strangers"
      title="People who aren’t saved contacts"
      description="Messaging strangers is a known reason WhatsApp bans accounts, and the easiest way for a tricked agent to send your data to someone. So it has its own setting."
    >
      <Field
        label="May it message numbers you haven’t saved"
        inputID={id}
        isGroupLabel
        description="Applies to numbers with no rule of their own above."
        status={
          strangers === 'allowed'
            ? { type: 'warning', message: WARNINGS.strangers }
            : undefined
        }
        statusVariant="detached"
      >
        <HStack>
          <SegmentedControl
            label="May it message numbers you haven’t saved"
            value={strangers}
            onChange={(next) =>
              onChange(updateChats(value, { strangers: next as Strangers }))
            }
          >
            <SegmentedControlItem value="never" label="Never" />
            <SegmentedControlItem value="ask" label="Ask me first" />
            <SegmentedControlItem value="allowed" label="Allowed" />
          </SegmentedControl>
        </HStack>
      </Field>
    </SettingsSection>
  )
}

function AccountSection({ value, onChange }: SectionProps) {
  // The selected chats are all it may know of, so they bound "All" too
  const reach =
    value.chats.scope === 'selected'
      ? ' “All” stops at the chats it may reach.'
      : ''
  return (
    <SettingsSection
      id="permissions-account"
      title="The account"
      description="What else it may see or change."
    >
      <Choice
        label="Find contacts and groups by name"
        description="So it can find “Ada” without being told her number. It gets at most 20 matches per search, and each search shows in the activity log."
        value={value.contacts}
        options={[
          ['none', 'Off'],
          ['find', 'On'],
        ]}
        onChange={(contacts) => onChange({ ...value, contacts })}
      />
      <Choice
        label="Scheduled messages"
        description={`Seeing, creating and changing messages that go out later.${reach}`}
        value={value.schedules}
        options={[
          ['none', 'None'],
          ['own', 'Its own'],
          ['all', 'All'],
        ]}
        warning={value.schedules === 'all' ? WARNINGS.schedulesAll : null}
        onChange={(schedules) => onChange({ ...value, schedules })}
      />
      <Choice
        label="Templates"
        description="Reading your saved templates. It can’t change them."
        value={value.templates}
        options={[
          ['none', 'Off'],
          ['read', 'Read'],
        ]}
        onChange={(templates) => onChange({ ...value, templates })}
      />
      <Choice
        label="History"
        description={`Seeing who a message went to, when, and whether it went out, as on your History page. Never its text.${reach}`}
        value={value.deliveries}
        options={[
          ['own', 'Its own messages'],
          ['all', 'All'],
        ]}
        onChange={(deliveries) => onChange({ ...value, deliveries })}
      />
    </SettingsSection>
  )
}

interface ChoiceProps<Value extends string> {
  label: string
  description: string
  value: Value
  options: ReadonlyArray<readonly [Value, string]>
  onChange: (value: Value) => void
  warning?: string | null
}

function Choice<Value extends string>({
  label,
  description,
  value,
  options,
  onChange,
  warning,
}: ChoiceProps<Value>) {
  const id = useId()
  return (
    <Field
      label={label}
      inputID={id}
      isGroupLabel
      description={description}
      status={warning ? { type: 'warning', message: warning } : undefined}
      statusVariant="detached"
    >
      <HStack>
        <SegmentedControl
          label={label}
          value={value}
          onChange={(next) => onChange(next as Value)}
        >
          {options.map(([option, text]) => (
            <SegmentedControlItem key={option} value={option} label={text} />
          ))}
        </SegmentedControl>
      </HStack>
    </Field>
  )
}
