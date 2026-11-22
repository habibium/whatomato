// Sits in front of the demo server and answers every /api/whatsapp request itself, so the pages
// show a linked account with made-up contacts while the server never opens a WhatsApp connection.
import { readFileSync } from 'node:fs'
import { createServer, request } from 'node:http'

const PORT = Number(process.env.DEMO_PORT ?? 8012)
const SERVER_PORT = Number(process.env.DEMO_SERVER_PORT ?? 8011)

// Ofcom keeps 07700 900000 to 900999 for drama, so none of these reach a real phone
const OWN_NUMBER = '447700900142'
const PAIR_CODE = 'K7QM2XVP'
const QR_CODE = `data:image/svg+xml;base64,${readFileSync(new URL('./pairing-qr.svg', import.meta.url)).toString('base64')}`

const STATES = [
  'connected',
  'not_linked',
  'pairing_qr',
  'pairing_code',
  'syncing',
  'reconnecting',
  'paused',
  'stopped',
]
const COOKIE = 'demo_whatsapp'

const contact = (name, phoneNumber, firstName = name.split(' ')[0]) => ({
  id: `${phoneNumber}@s.whatsapp.net`,
  name,
  first_name: firstName,
  phone_number: phoneNumber,
  username: null,
})

const CONTACTS = [
  contact('Aisha Rahman', '447700900101'),
  contact('Ben Whitaker', '447700900115'),
  contact('Carlos Mendes', '447700900128'),
  contact('Dr Priya Patel', '447700900133', 'Priya'),
  contact('Elena Petrova', '447700900147'),
  contact('Farid Hossain', '447700900152'),
  contact('Grace Liu', '447700900158'),
  contact('Hannah Schmidt', '447700900166'),
  contact('Imran Chowdhury', '447700900171'),
  contact('Jonas Berg', '447700900189'),
  // WhatsApp hasn't shared this one's number, so nothing can be sent to it
  {
    id: '84712290541137@lid',
    name: 'Leo Martin',
    first_name: 'Leo',
    phone_number: null,
    username: null,
  },
  contact('Mum', '447700900104'),
  contact('Olivia Bennett', '447700900112'),
  contact('Sam Taylor', '447700900193'),
  contact('Tomás García', '447700900120'),
  contact('Yuki Tanaka', '447700900177'),
]

const GROUPS = [
  { id: '120363041877203345@g.us', name: 'Book Club' },
  { id: '120363025518846012@g.us', name: 'Family' },
  { id: '120363047730195528@g.us', name: 'Flat 4B' },
  { id: '120363039264410877@g.us', name: 'Product Team' },
  { id: '120363044096627301@g.us', name: 'Sunday Football' },
  { id: '120363032205581469@g.us', name: 'Year 6 Parents' },
]

function stateOf(req) {
  const match = (req.headers.cookie ?? '').match(new RegExp(`${COOKIE}=([a-z_]+)`))
  return match && STATES.includes(match[1]) ? match[1] : 'connected'
}

function status(state) {
  // Always a little ahead, so a code never shows as expired however long the page stays open
  const expiresAt = new Date(Date.now() + 45_000).toISOString()
  switch (state) {
    case 'not_linked':
      return { status: 'not_linked', reason: null }
    case 'pairing_qr':
      return {
        status: 'pairing',
        method: 'qr_code',
        phone_number: null,
        qr_code: QR_CODE,
        pair_code: null,
        expires_at: expiresAt,
      }
    case 'pairing_code':
      return {
        status: 'pairing',
        method: 'phone_number',
        phone_number: OWN_NUMBER,
        qr_code: null,
        pair_code: PAIR_CODE,
        expires_at: expiresAt,
      }
    case 'stopped':
      return { status: 'stopped', phone_number: OWN_NUMBER, reason: 'replaced', until: null }
    default:
      return { status: state, phone_number: OWN_NUMBER }
  }
}

const isLinked = (state) => !['not_linked', 'pairing_qr', 'pairing_code'].includes(state)

function send(res, code, body, headers = {}) {
  if (body === undefined) {
    res.writeHead(code, headers).end()
    return
  }
  res.writeHead(code, { 'content-type': 'application/json', ...headers }).end(JSON.stringify(body))
}

const setState = (state) => ({ 'set-cookie': `${COOKIE}=${state}; Path=/; SameSite=Lax` })

async function json(req) {
  const chunks = []
  for await (const chunk of req) chunks.push(chunk)
  try {
    return JSON.parse(Buffer.concat(chunks).toString() || '{}')
  } catch {
    return {}
  }
}

async function whatsapp(req, res, path) {
  const state = stateOf(req)
  const route = `${req.method} ${path}`
  switch (route) {
    case 'GET /status':
      return send(res, 200, { ...status(state), can_link: true })
    case 'GET /contacts':
      return send(res, 200, isLinked(state)
        ? { imported: state !== 'syncing', contacts: state === 'syncing' ? [] : CONTACTS }
        : { imported: false, contacts: [] })
    case 'GET /groups':
      return state === 'connected'
        ? send(res, 200, GROUPS)
        : send(res, 409, { error: "WhatsApp isn't connected" })
    case 'POST /link': {
      const { phone_number: phoneNumber } = await json(req)
      return send(res, 204, undefined, setState(phoneNumber ? 'pairing_code' : 'pairing_qr'))
    }
    case 'POST /link/cancel':
      return send(res, 204, undefined, setState('not_linked'))
    case 'POST /pause':
      return send(res, 204, undefined, setState('paused'))
    case 'POST /resume':
      return send(res, 204, undefined, setState('connected'))
    case 'POST /unlink':
      return send(res, 200, { removed_from_phone: true }, setState('not_linked'))
    case 'POST /test-message':
      return send(res, 204)
    default:
      // Pictures included: every contact shows its initials
      return send(res, 404, { error: 'Not found' })
  }
}

function forward(req, res) {
  const upstream = request(
    { host: '127.0.0.1', port: SERVER_PORT, method: req.method, path: req.url, headers: req.headers },
    (answer) => {
      res.writeHead(answer.statusCode, answer.headers)
      answer.pipe(res)
    },
  )
  upstream.on('error', () => {
    if (!res.headersSent) res.writeHead(502, { 'content-type': 'text/plain' })
    res.end(`The demo server isn't answering on port ${SERVER_PORT}.`)
  })
  req.pipe(upstream)
}

createServer((req, res) => {
  const { pathname } = new URL(req.url, 'http://localhost')

  // /__demo/whatsapp/paused makes this browser, and no other, see a paused account
  const chosen = pathname.match(/^\/__demo\/whatsapp\/([a-z_]+)$/)
  if (chosen) {
    if (!STATES.includes(chosen[1])) {
      res.writeHead(404, { 'content-type': 'text/plain' }).end(`One of: ${STATES.join(', ')}`)
      return
    }
    res
      .writeHead(200, { 'content-type': 'text/plain', ...setState(chosen[1]) })
      .end(`WhatsApp shows as ${chosen[1]} in this browser.`)
    return
  }

  // What the owner keeps is the server's own setting, so it answers that
  if (pathname.startsWith('/api/whatsapp/') && pathname !== '/api/whatsapp/history') {
    whatsapp(req, res, pathname.slice('/api/whatsapp'.length))
    return
  }
  forward(req, res)
}).listen(PORT, '127.0.0.1', () => {
  console.log(`demo: http://localhost:${PORT} (server on ${SERVER_PORT})`)
})
