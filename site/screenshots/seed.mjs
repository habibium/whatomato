// Fills an empty demo database with one account and the made-up data the screenshots show.
// Run by demo.sh once the demo server and proxy.mjs are up.
import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'

const BASE = `http://localhost:${process.env.DEMO_PORT ?? 8012}`
const DATABASE_URL = process.env.DEMO_DATABASE_URL
const EMAIL = 'maya@example.com'
const NEW_EMAIL = 'alex@example.com'
// Only ever unlocks a throwaway account on this machine
const PASSWORD = 'tomato-ladder-quartz-91'
const ZONE = 'Europe/London'

// The seed deletes its account before making it again, so it must never be pointed anywhere else
if (!DATABASE_URL?.endsWith('/whatomato_docs_demo')) {
  throw new Error('DEMO_DATABASE_URL must name the whatomato_docs_demo database')
}

const sql = (query) =>
  execFileSync('psql', [DATABASE_URL, '-v', 'ON_ERROR_STOP=1', '-qAtc', query], {
    encoding: 'utf8',
  }).trim()

let cookie = ''

async function api(method, path, body) {
  const response = await fetch(BASE + path, {
    method,
    headers: { 'content-type': 'application/json', cookie },
    body: body === undefined ? undefined : JSON.stringify(body),
  })
  const text = await response.text()
  if (!response.ok) throw new Error(`${method} ${path}: ${response.status} ${text}`)
  const session = response.headers.getSetCookie().map((line) => line.split(';')[0])
  if (session.length) cookie = session.join('; ')
  return text ? JSON.parse(text) : null
}

let rpcId = 0

/** One MCP tool call as `key`'s connection; a refusal comes back as text, like any other answer. */
async function tool(key, name, args = {}) {
  const response = await fetch(`${BASE}/mcp`, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      accept: 'application/json, text/event-stream',
      authorization: `Bearer ${key}`,
    },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id: ++rpcId,
      method: 'tools/call',
      params: { name, arguments: args },
    }),
  })
  const text = await response.text()
  if (!response.ok) throw new Error(`${name}: ${response.status} ${text}`)
  const payload = text.startsWith('{')
    ? text
    : text
        .split('\n')
        .filter((line) => line.startsWith('data:'))
        .map((line) => line.slice(5))
        .join('')
  const { result, error } = JSON.parse(payload)
  if (error) throw new Error(`${name}: ${JSON.stringify(error)}`)
  const answer = result.structuredContent ?? result.content?.[0]?.text
  console.log(`  ${name}: ${result.isError ? 'refused' : (answer?.outcome ?? 'ok')}`)
  return answer
}

const contact = (name, number) => ({
  recipient_type: 'contact',
  recipient: number,
  recipient_name: name,
})
const group = (name, id) => ({ recipient_type: 'group', recipient: id, recipient_name: name })

// The same people and groups proxy.mjs answers with
const JONAS = contact('Jonas Berg', '447700900189')
const MUM = contact('Mum', '447700900104')
const AISHA = contact('Aisha Rahman', '447700900101')
const PRIYA = contact('Dr Priya Patel', '447700900133')
const SAM = contact('Sam Taylor', '447700900193')
const OLIVIA = contact('Olivia Bennett', '447700900112')
const BEN = contact('Ben Whitaker', '447700900115')
const FAMILY = group('Family', '120363025518846012@g.us')
const FLAT = group('Flat 4B', '120363047730195528@g.us')
const TEAM = group('Product Team', '120363039264410877@g.us')
const FOOTBALL = group('Sunday Football', '120363044096627301@g.us')

const cron = (expression) => ({
  schedule_kind: 'recurring',
  recurrence: { type: 'cron', expression },
  timezone: ZONE,
})

const daysFromNow = (days, hour, minute = 0) => {
  const at = new Date()
  at.setUTCDate(at.getUTCDate() + days)
  at.setUTCHours(hour, minute, 0, 0)
  return at.toISOString()
}

console.log('accounts')
// The second one stays empty, for the pages a new account sees
for (const email of [NEW_EMAIL, EMAIL]) {
  sql(`DELETE FROM users WHERE email = '${email}'`)
  await api('POST', '/api/auth/signup', { email, password: PASSWORD })
  sql(`UPDATE users SET verified_at = now() WHERE email = '${email}'`)
}
await api('POST', '/api/auth/signin', { email: EMAIL, password: PASSWORD })

console.log('templates')
const templates = {}
for (const [name, body] of [
  [
    'Payment reminder',
    'Hi {{first_name|there}}, a quick reminder that this month’s invoice is due on {{date + 7d}}. Thank you!',
  ],
  ['Running late', 'Running about 10 minutes late, sorry! See you soon.'],
  [
    'Weekly check-in',
    'Hi {{first_name}}, how did this week go? Anything you need from me before {{day + 3d}}?',
  ],
  ['Appointment confirmation', 'Hi {{name}}, confirming our appointment on {{date + 1d}}. See you then.'],
]) {
  templates[name] = await api('POST', '/api/templates', { name, body })
}

console.log('scheduled messages')
const message = (to, body, schedule, extra = {}) =>
  api('POST', '/api/messages', { ...to, body, ...schedule, ...extra })

await message(
  TEAM,
  'Good morning team ☀️ Standup is at 9:30. Post your blockers here before then.',
  cron('0 9 * * 1-5'),
)
await message(
  JONAS,
  'Hi Jonas, the rent for {{month}} is on its way. Let me know once it lands.',
  cron('0 8 1 * *'),
)
await message(
  FOOTBALL,
  'Kick-off is at 10 tomorrow. Reply IN or OUT so we can sort the teams.',
  cron('0 18 * * 6'),
)
await message(FAMILY, 'Sunset is at {{sunset}} today. Who’s coming for the evening walk?', {
  schedule_kind: 'recurring',
  recurrence: { type: 'sun', event: 'sunset', offset_minutes: -45, days: '* * *', place: null },
  timezone: ZONE,
})
await message(MUM, 'Happy birthday, Mum! 🎂 I’ll call you this evening.', {
  schedule_kind: 'once',
  run_at: daysFromNow(9, 8),
  timezone: ZONE,
})
await message(
  AISHA,
  templates['Weekly check-in'].body,
  cron('0 16 * * 5'),
  { template_id: templates['Weekly check-in'].id },
)
await message(
  FLAT,
  'Bin day tomorrow: recycling goes out tonight.',
  cron('0 19 * * 3'),
  { enabled: false },
)

console.log('agent connections')
const presets = Object.fromEntries(
  (await api('GET', '/api/agents/presets')).map((preset) => [preset.id, preset.grant]),
)
const connect = (name, grant, extra = {}) =>
  api('POST', '/api/agents', { name, grant, password: PASSWORD, ...extra })

const claude = await connect('Claude', {
  ...presets.ask_before_sending,
  attachments: { open: false, send: true },
})

const rule = (send, note = null, dailyLimit = null) => ({
  send,
  daily_limit: dailyLimit,
  note,
  read: false,
})
const briefing = await connect('Morning briefing', {
  ...presets.ask_before_sending,
  schedules: 'own',
  chats: {
    scope: 'selected',
    default: rule('no'),
    strangers: 'never',
    rules: [
      {
        chat: { type: 'group', id: TEAM.recipient, name: TEAM.recipient_name },
        ...rule('yes', 'Work chat. Keep it short and factual.', 5),
      },
      {
        chat: { type: 'contact', id: SAM.recipient, name: SAM.recipient_name },
        ...rule('ask', 'My manager. Always check with me first.'),
      },
    ],
  },
})

const laptop = await connect('Old laptop', presets.view_only, { expires_in_days: 30 })
await api('POST', `/api/agents/${laptop.connection.id}/pause`)

console.log('what the agents did')
await tool(claude.key, 'whoami')
await tool(claude.key, 'list_templates')
await tool(claude.key, 'find_chats', { query: 'jonas' })
await tool(claude.key, 'list_scheduled_messages')
const rejected = await tool(claude.key, 'send_message', {
  ...PRIYA,
  body: 'Hi Dr Patel, could I move my check-up on Thursday to the following week?',
  timezone: ZONE,
})
// A contact, not a group: an agent's request carries no name, and the server can't look a group's
// up while nothing is connected
await tool(claude.key, 'schedule_message', {
  ...OLIVIA,
  body: 'Reminder: Grandad’s 80th is on Saturday. Lunch is at 1 at ours.',
  schedule_kind: 'once',
  run_at: daysFromNow(3, 9),
  timezone: ZONE,
})
await tool(claude.key, 'send_message', {
  ...JONAS,
  body: 'Hi Jonas, the boiler is making that noise again. Could someone take a look this week?',
  timezone: ZONE,
  // Photo by Salah Ait Mokhtar on Unsplash
  attachment: {
    data: readFileSync(new URL('boiler.jpg', import.meta.url)).toString('base64'),
    file_name: 'boiler.jpg',
    mime_type: 'image/jpeg',
  },
})

await tool(briefing.key, 'whoami')
await tool(briefing.key, 'schedule_message', {
  ...TEAM,
  body: 'Friday wrap-up: reply with one thing you shipped this week.',
  ...cron('0 16 * * 5'),
})
// Outside its chats, so the activity log has a refusal to show
await tool(briefing.key, 'send_message', {
  ...BEN,
  body: 'Good morning! Here is today’s briefing.',
})
await tool(briefing.key, 'send_message', {
  ...SAM,
  body: 'Morning Sam, the Q4 numbers are in the shared folder. Anything you want changed before 11?',
  timezone: ZONE,
})

if (rejected?.approval_id) {
  await api('POST', `/api/agents/approvals/${rejected.approval_id}/reject`)
}

console.log('history')
const userId = sql(`SELECT id FROM users WHERE email = '${EMAIL}'`)
const text = (value) => (value === null ? 'NULL' : `'${String(value).replaceAll("'", "''")}'`)

// Moments in the demo's time zone, written as SQL: so many days before today, at a time of day
const at = (daysAgo, time) =>
  `(((now() AT TIME ZONE '${ZONE}')::date - ${daysAgo}) + time '${time}') AT TIME ZONE '${ZONE}'`
const weekday = new Date(new Date().toLocaleString('en-US', { timeZone: ZONE })).getDay()
/** How many days ago the latest such weekday was, today not counted; Sunday is 0 */
const daysSince = (day) => ((weekday - day + 6) % 7) + 1
const [lastWorkday, workdayBefore] = [1, 2, 3, 4, 5, 6, 7].filter((days) => {
  const day = (weekday - days + 14) % 7
  return day >= 1 && day <= 5
})
const FRIDAY = 5
const SATURDAY = 6

// A one-time message that has gone out, for the Past tab
const pastId = sql(`
  INSERT INTO scheduled_messages
    (user_id, recipient_type, recipient, recipient_name, body, schedule_kind, run_at, timezone,
     enabled, next_run_at, last_run_at, created_at, updated_at)
  VALUES
    ('${userId}', 'contact', '${SAM.recipient}', ${text(SAM.recipient_name)},
     'Hi Sam, the slides for tomorrow are in the shared folder.', 'once',
     ${at(2, '17:30')}, '${ZONE}', true, NULL, ${at(2, '17:30')}, ${at(4, '10:12')},
     ${at(2, '17:30')})
  RETURNING id`)

const scheduleOf = (body) =>
  sql(`SELECT id FROM scheduled_messages WHERE user_id = '${userId}' AND body LIKE ${text(`${body}%`)} LIMIT 1`)
const standup = scheduleOf('Good morning team')
const football = scheduleOf('Kick-off')
const checkIn = scheduleOf('Hi {{first_name}}, how did this week go')
const walk = scheduleOf('Sunset is at')

const STANDUP = 'Good morning team ☀️ Standup is at 9:30. Post your blockers here before then.'
const deliveries = [
  [TEAM, STANDUP, at(lastWorkday, '09:00'), standup],
  [FAMILY, 'Sunset is at 4:09 PM today. Who’s coming for the evening walk?', at(1, '15:24'), walk],
  [PRIYA, 'Running about 10 minutes late, sorry! See you soon.', at(1, '11:52'), null],
  [TEAM, STANDUP, at(workdayBefore, '09:00'), standup],
  [SAM, 'Hi Sam, the slides for tomorrow are in the shared folder.', at(2, '17:30'), pastId],
  [
    TEAM,
    'Friday wrap-up: reply with one thing you shipped this week.',
    at(daysSince(FRIDAY), '16:00'),
    null,
    { connectionId: briefing.connection.id },
  ],
  [
    AISHA,
    'Hi Aisha, how did this week go? Anything you need from me before Monday?',
    at(daysSince(FRIDAY), '16:00'),
    checkIn,
    { error: "WhatsApp wasn't connected" },
  ],
  [
    FOOTBALL,
    'Kick-off is at 10 tomorrow. Reply IN or OUT so we can sort the teams.',
    at(daysSince(SATURDAY), '18:00'),
    football,
  ],
  [
    JONAS,
    'Hi Jonas, are you free for a quick call about the lease renewal this afternoon?',
    at(6, '14:10'),
    null,
    { connectionId: claude.connection.id },
  ],
]
for (const [to, body, when, scheduleId, { connectionId = null, error = null } = {}] of deliveries) {
  sql(`
    INSERT INTO deliveries
      (user_id, scheduled_message_id, connection_id, recipient_type, recipient, recipient_name,
       body, status, attempts, error, created_at, sent_at, next_attempt_at)
    VALUES
      ('${userId}', ${text(scheduleId)}, ${text(connectionId)}, '${to.recipient_type}',
       '${to.recipient}', ${text(to.recipient_name)}, ${text(body)}, '${error ? 'failed' : 'sent'}',
       ${error ? 3 : 1}, ${text(error)}, ${when},
       ${error ? 'NULL' : `${when} + interval '2 seconds'`}, ${when})`)
}

// Nothing can go out without WhatsApp, so a message that comes due while the demo is up is
// recorded as sent instead of filling History with failures
sql(`
  CREATE OR REPLACE FUNCTION demo_send() RETURNS trigger AS $$
  BEGIN
    NEW.status := 'sent';
    NEW.attempts := 1;
    NEW.sent_at := NEW.created_at + interval '2 seconds';
    RETURN NEW;
  END
  $$ LANGUAGE plpgsql;
  DROP TRIGGER IF EXISTS demo_send ON deliveries;
  CREATE TRIGGER demo_send BEFORE INSERT ON deliveries
    FOR EACH ROW WHEN (NEW.status = 'pending') EXECUTE FUNCTION demo_send();`)

console.log(`done: sign in at ${BASE} as ${EMAIL} with ${PASSWORD}`)
