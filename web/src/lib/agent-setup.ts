/** Stands in for the key wherever it can't be shown, such as on a connection's page after creation. */
export const KEY_PLACEHOLDER = 'YOUR_ACCESS_KEY'

/** The name the agent lists the server under, in its own configuration. */
const SERVER_NAME = 'whatomato'
const KEY_VARIABLE = 'WHATOMATO_KEY'

export type AgentClientID =
  | 'claude-code'
  | 'codex'
  | 'opencode'
  | 'antigravity'
  | 'cursor'
  | 'other'

export interface SetupStep {
  /** What to do, in one sentence. */
  text: string
  /** What to paste or run; empty for a step that is only words. */
  code?: string
  language?: 'bash' | 'json' | 'toml' | 'plaintext'
  /** The file the code goes in, when there is one. */
  file?: string
  /** The code's header when it goes in no file; without a header the copy button covers the first line. */
  title?: string
}

export interface AgentClient {
  id: AgentClientID
  name: string
}

export const AGENT_CLIENTS: readonly AgentClient[] = [
  { id: 'claude-code', name: 'Claude Code' },
  { id: 'codex', name: 'Codex' },
  { id: 'opencode', name: 'OpenCode' },
  { id: 'antigravity', name: 'Antigravity' },
  { id: 'cursor', name: 'Cursor' },
  { id: 'other', name: 'Something else' },
]

/**
 * The steps to connect one client, with the endpoint and the key filled in; without a key they show
 * KEY_PLACEHOLDER. Shapes follow each client's documentation as read in docs/agent-access/research/clients.md.
 */
export function setupSteps(
  client: AgentClientID,
  { url, accessKey }: { url: string; accessKey?: string },
): SetupStep[] {
  const key = accessKey ?? KEY_PLACEHOLDER
  const bearer = `Bearer ${key}`
  const json = (value: unknown) => JSON.stringify(value, null, 2)

  switch (client) {
    case 'claude-code':
      return [
        {
          text: 'Run this in a terminal.',
          title: 'Terminal',
          code: `claude mcp add --scope user --transport http ${SERVER_NAME} ${url} \\\n  --header "Authorization: ${bearer}"`,
          language: 'bash',
        },
        {
          text: 'Start Claude Code and type /mcp to check that it connected.',
        },
      ]
    case 'codex':
      return [
        {
          text: 'Add this to Codex’s configuration file.',
          file: '~/.codex/config.toml',
          code: `[mcp_servers.${SERVER_NAME}]\nurl = "${url}"\nbearer_token_env_var = "${KEY_VARIABLE}"`,
          language: 'toml',
        },
        {
          text: 'Codex reads the key from your environment. Add this line to your shell’s profile, such as ~/.zshrc or ~/.bashrc, then open a new terminal.',
          title: 'Shell profile',
          code: `export ${KEY_VARIABLE}="${key}"`,
          language: 'bash',
        },
      ]
    case 'opencode':
      return [
        {
          text: 'Add this to OpenCode’s configuration file, next to anything already under "mcp" → "servers". Keep "oauth": false, or OpenCode tries a sign-in Whatomato doesn’t offer.',
          file: 'opencode.json',
          code: json({
            mcp: {
              servers: {
                [SERVER_NAME]: {
                  type: 'remote',
                  url,
                  oauth: false,
                  headers: { Authorization: bearer },
                },
              },
            },
          }),
          language: 'json',
        },
      ]
    case 'antigravity':
      return [
        {
          text: 'Add this to Antigravity’s configuration file, next to anything already under "mcpServers".',
          file: '~/.gemini/config/mcp_config.json',
          code: json({
            mcpServers: {
              [SERVER_NAME]: {
                serverUrl: url,
                headers: { Authorization: bearer },
              },
            },
          }),
          language: 'json',
        },
      ]
    case 'cursor':
      return [
        {
          text: 'Add this to Cursor’s configuration file, next to anything already under "mcpServers".',
          file: '~/.cursor/mcp.json',
          code: json({
            mcpServers: {
              [SERVER_NAME]: { url, headers: { Authorization: bearer } },
            },
          }),
          language: 'json',
        },
      ]
    case 'other':
      return [
        {
          text: 'In your agent, add a remote MCP server at this address. It speaks Streamable HTTP.',
          title: 'Address',
          code: url,
          language: 'plaintext',
        },
        {
          text: 'Send this header with every request. The header’s name is Authorization.',
          title: 'Header',
          code: `Authorization: ${bearer}`,
          language: 'plaintext',
        },
      ]
  }
}
