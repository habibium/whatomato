-- An AI agent the user has let into their account through the MCP endpoint, and what it may do.
-- Only the key's hash is kept; the key is shown once. `grant` is the server's versioned `Grant`, so
-- a new permission needs no migration. The 20-per-user cap is enforced by the insert itself.
CREATE TABLE agent_connections (
	id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
	user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
	name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 60),
	key_hash BYTEA NOT NULL UNIQUE,
	key_hint TEXT NOT NULL CHECK (char_length(key_hint) = 4),
	"grant" JSONB NOT NULL,
	paused_at TIMESTAMPTZ,
	expires_at TIMESTAMPTZ,
	last_used_at TIMESTAMPTZ,
	created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
	updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX agent_connections_user_id ON agent_connections (user_id, created_at);

-- Set null rather than cascade: revoking an agent keeps the history of what it sent, and its
-- schedules unless the owner asks for them to go too
ALTER TABLE deliveries
	ADD COLUMN connection_id UUID REFERENCES agent_connections (id) ON DELETE SET NULL;
ALTER TABLE scheduled_messages
	ADD COLUMN connection_id UUID REFERENCES agent_connections (id) ON DELETE SET NULL;

-- Partial, since nearly every row was made by the owner; they serve the daily counts and "its own"
CREATE INDEX deliveries_connection_id ON deliveries (connection_id, created_at)
	WHERE connection_id IS NOT NULL;
CREATE INDEX scheduled_messages_connection_id ON scheduled_messages (connection_id)
	WHERE connection_id IS NOT NULL;

-- What an agent asked to do that waits for the owner. `payload` is the request sealed with `Cipher`
-- and bound to the row's id, so its text never sits in the clear.
CREATE TABLE agent_approvals (
	id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
	connection_id UUID NOT NULL REFERENCES agent_connections (id) ON DELETE CASCADE,
	user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
	kind TEXT NOT NULL CHECK (kind IN ('send', 'schedule_create', 'schedule_update')),
	recipient_type TEXT NOT NULL CHECK (recipient_type IN ('contact', 'group')),
	recipient TEXT NOT NULL,
	recipient_name TEXT,
	payload BYTEA NOT NULL,
	status TEXT NOT NULL DEFAULT 'waiting'
		CHECK (status IN ('waiting', 'approved', 'rejected', 'expired')),
	delivery_id UUID REFERENCES deliveries (id) ON DELETE SET NULL,
	scheduled_message_id UUID REFERENCES scheduled_messages (id) ON DELETE SET NULL,
	created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
	expires_at TIMESTAMPTZ NOT NULL DEFAULT now() + interval '24 hours',
	decided_at TIMESTAMPTZ,
	CHECK ((status = 'waiting') = (decided_at IS NULL))
);

CREATE INDEX agent_approvals_user_id ON agent_approvals (user_id, created_at DESC);
CREATE INDEX agent_approvals_connection_id ON agent_approvals (connection_id, created_at DESC);
CREATE INDEX agent_approvals_waiting ON agent_approvals (expires_at) WHERE status = 'waiting';

-- One row per tool call, kept 90 days. No message text. The connection's name is copied so the
-- log still reads after a revoke; the recipient columns are empty when no chat is involved.
CREATE TABLE agent_activity (
	id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
	user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
	connection_id UUID REFERENCES agent_connections (id) ON DELETE SET NULL,
	connection_name TEXT NOT NULL,
	tool TEXT NOT NULL,
	recipient_type TEXT CHECK (recipient_type IN ('contact', 'group')),
	recipient TEXT,
	recipient_name TEXT,
	outcome TEXT NOT NULL CHECK (outcome IN ('allowed', 'asked', 'denied', 'failed')),
	reason TEXT,
	created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
	CHECK ((recipient_type IS NULL) = (recipient IS NULL))
);

CREATE INDEX agent_activity_user_id ON agent_activity (user_id, created_at DESC);
CREATE INDEX agent_activity_connection_id ON agent_activity (connection_id, created_at DESC);
CREATE INDEX agent_activity_created_at ON agent_activity (created_at);
