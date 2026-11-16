import type { components } from '@/api/schema'

import type { AgentPreset } from '@/lib/agents'

export type Grant = components['schemas']['Grant']
export type Rule = components['schemas']['Rule']
export type ChatRule = components['schemas']['ChatRule']
export type Chat = components['schemas']['Chat']
export type SendLevel = components['schemas']['SendLevel']
export type Strangers = components['schemas']['Strangers']
export type Scope = components['schemas']['Scope']

/** The server's caps; it refuses a grant past them. */
export const MAX_CHAT_RULES = 500
export const NOTE_MAX = 500

export function chatKey(chat: Pick<Chat, 'type' | 'id'>) {
  return `${chat.type}:${chat.id}`
}

/** Exactly the fields the server takes: it refuses a grant with any it doesn't know. */
export function cleanGrant(grant: Grant): Grant {
  const rule = (value: Rule): Rule => ({
    read: value.read ?? false,
    send: value.send,
    daily_limit: value.daily_limit ?? null,
    note: value.note?.trim() ? value.note : null,
  })
  return {
    version: 1,
    contacts: grant.contacts,
    schedules: grant.schedules,
    templates: grant.templates,
    deliveries: grant.deliveries,
    chats: {
      scope: grant.chats.scope,
      default: rule(grant.chats.default),
      rules: grant.chats.rules.map((item) => ({
        chat: {
          type: item.chat.type,
          id: item.chat.id,
          name: item.chat.name ?? null,
        },
        ...rule(item),
      })),
      strangers: grant.chats.strangers,
    },
  }
}

/** Rule order and a chat's label don't change what a grant allows. */
function canonical(grant: Grant) {
  const clean = cleanGrant(grant)
  const rules = clean.chats.rules
    .map((item) => ({
      ...item,
      chat: { type: item.chat.type, id: item.chat.id },
    }))
    .sort((a, b) => chatKey(a.chat).localeCompare(chatKey(b.chat)))
  return JSON.stringify({ ...clean, chats: { ...clean.chats, rules } })
}

export function sameGrant(a: Grant, b: Grant) {
  return canonical(a) === canonical(b)
}

/** Reading is chosen on its own, so it doesn't turn a preset into custom permissions. */
function withoutReading(grant: Grant): Grant {
  return {
    ...grant,
    chats: {
      ...grant.chats,
      default: { ...grant.chats.default, read: false },
      rules: grant.chats.rules.map((item) => ({ ...item, read: false })),
    },
  }
}

export function matchingPreset(grant: Grant, presets: readonly AgentPreset[]) {
  const sending = withoutReading(grant)
  return presets.find((preset) => sameGrant(sending, preset.grant)) ?? null
}

/** Whether it may read at least one chat. */
export function readsAny(grant: Grant) {
  const { scope, rules } = grant.chats
  return (
    (scope === 'all' && grant.chats.default.read === true) ||
    rules.some((item) => item.read === true)
  )
}

export function findChatRule(grant: Grant, key: string) {
  return grant.chats.rules.find((item) => chatKey(item.chat) === key) ?? null
}

/** A chat the owner just added never starts able to send without asking. */
export function newChatSend(send: SendLevel): SendLevel {
  return send === 'yes' ? 'ask' : send
}

/** A new chat starts from the default rule; one already listed, or one past the cap, changes nothing. */
export function addChatRule(grant: Grant, chat: Chat): Grant {
  const rules = grant.chats.rules
  if (rules.length >= MAX_CHAT_RULES || findChatRule(grant, chatKey(chat))) {
    return grant
  }
  const { read, send, daily_limit, note } = grant.chats.default
  return {
    ...grant,
    chats: {
      ...grant.chats,
      rules: [
        ...rules,
        {
          chat,
          read: read ?? false,
          send: newChatSend(send),
          daily_limit,
          note,
        },
      ],
    },
  }
}

export function removeChatRule(grant: Grant, key: string): Grant {
  return {
    ...grant,
    chats: {
      ...grant.chats,
      rules: grant.chats.rules.filter((item) => chatKey(item.chat) !== key),
    },
  }
}

export function updateChatRule(
  grant: Grant,
  key: string,
  change: Partial<Rule>,
): Grant {
  return {
    ...grant,
    chats: {
      ...grant.chats,
      rules: grant.chats.rules.map((item) =>
        chatKey(item.chat) === key ? { ...item, ...change } : item,
      ),
    },
  }
}

export function updateDefaultRule(grant: Grant, change: Partial<Rule>): Grant {
  return {
    ...grant,
    chats: { ...grant.chats, default: { ...grant.chats.default, ...change } },
  }
}

export function updateChats(
  grant: Grant,
  change: Partial<Pick<Grant['chats'], 'scope' | 'strangers'>>,
): Grant {
  return { ...grant, chats: { ...grant.chats, ...change } }
}

export const SEND_LABELS: Record<SendLevel, string> = {
  no: 'No',
  ask: 'Ask me first',
  yes: 'Yes',
}

/** "Asks you first", "Sends without asking, up to 5 a day". */
export function describeRule(rule: Rule) {
  const base =
    rule.send === 'no'
      ? "Can't send"
      : rule.send === 'ask'
        ? 'Asks you first'
        : 'Sends without asking'
  const sends =
    rule.send !== 'no' && rule.daily_limit !== null
      ? `${base}, up to ${rule.daily_limit} a day`
      : base
  return rule.read ? `Reads messages · ${sends}` : sends
}

/** "All chats", "All chats, 2 with their own rule", "3 chats". */
export function describeChats(grant: Grant) {
  const count = grant.chats.rules.length
  if (grant.chats.scope === 'selected') {
    return count === 0
      ? 'No chats'
      : `${count} ${count === 1 ? 'chat' : 'chats'}`
  }
  return count === 0
    ? 'All chats'
    : `All chats, ${count} with ${count === 1 ? 'its' : 'their'} own rule`
}

/** Whether any chat it can reach may be sent to without the owner seeing the message first. */
export function sendsWithoutAsking(grant: Grant) {
  const { scope, default: fallback, rules, strangers } = grant.chats
  return (
    (scope === 'all' && fallback.send === 'yes') ||
    rules.some((item) => item.send === 'yes') ||
    strangers === 'allowed'
  )
}

export const WARNINGS = {
  readAndSend:
    'It reads what other people write and can send without asking, so a message written to trick it could make it send under your name. “Ask me first” keeps you in the loop.',
  sendDefault:
    'It can send to any chat without showing you the message first, so a mistake or a tricked agent goes out under your name.',
  sendChat:
    'It can send here without showing you the message first, so a mistake goes out under your name.',
  strangers:
    'An agent tricked by something it reads could send your messages or contacts to a number you’ve never saved.',
  schedulesAll:
    'It can change or delete the messages you scheduled yourself, not only its own.',
} as const

/** The one warning a whole set of permissions deserves, strongest first. */
export function grantWarning(
  grant: Grant,
): { type: 'error' | 'warning'; message: string } | null {
  if (grant.chats.strangers === 'allowed') {
    return {
      type: 'error',
      message:
        'It can message anyone, including numbers you’ve never saved, without asking you. An agent tricked by something it reads could send your data to a stranger.',
    }
  }
  if (sendsWithoutAsking(grant)) {
    return {
      type: 'warning',
      message:
        'It sends without showing you the message first, so a mistake or a tricked agent goes out under your name.',
    }
  }
  return null
}
