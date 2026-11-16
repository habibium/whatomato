-- A connection is made with an access key or by signing in (OAuth). One made by signing in has no
-- key; its credentials are rows of agent_tokens. `client_id` is the URL of the client's metadata
-- document, and `client_name` is what that document called itself, so neither is to be trusted.
ALTER TABLE agent_connections
	ALTER COLUMN key_hash DROP NOT NULL,
	ALTER COLUMN key_hint DROP NOT NULL,
	ADD COLUMN client_id TEXT CHECK (char_length(client_id) <= 2048),
	ADD COLUMN client_name TEXT CHECK (char_length(client_name) <= 120),
	ADD CONSTRAINT agent_connections_credential CHECK (
		(key_hash IS NULL) = (key_hint IS NULL)
		AND (client_id IS NULL) = (client_name IS NULL)
		AND (key_hash IS NULL) <> (client_id IS NULL)
	);

-- An authorization code: the owner said yes on the consent page, and the client has a minute to
-- exchange it. It carries either the connection to sign in to again, or the name and permissions
-- of a new one, which is created only at the exchange. A used row stays until it expires, with
-- the connection it led to, so a second use can sign that connection out.
CREATE TABLE agent_sign_ins (
	id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
	code_hash BYTEA NOT NULL UNIQUE,
	user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
	client_id TEXT NOT NULL,
	client_name TEXT NOT NULL,
	redirect_uri TEXT NOT NULL,
	code_challenge TEXT NOT NULL,
	connection_id UUID REFERENCES agent_connections (id) ON DELETE CASCADE,
	name TEXT,
	"grant" JSONB,
	used_at TIMESTAMPTZ,
	expires_at TIMESTAMPTZ NOT NULL,
	created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
	CHECK (used_at IS NOT NULL OR (connection_id IS NULL) <> (name IS NULL)),
	CHECK ((name IS NULL) = ("grant" IS NULL))
);

CREATE INDEX agent_sign_ins_expires_at ON agent_sign_ins (expires_at);

-- One access token and the refresh token issued with it; only hashes are kept. Using the refresh
-- token sets `replaced_at` and adds a new row. A replaced refresh token shown again, past a short
-- grace, means it leaked, and every row of the connection goes.
CREATE TABLE agent_tokens (
	id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
	connection_id UUID NOT NULL REFERENCES agent_connections (id) ON DELETE CASCADE,
	access_hash BYTEA NOT NULL UNIQUE,
	refresh_hash BYTEA NOT NULL UNIQUE,
	access_expires_at TIMESTAMPTZ NOT NULL,
	refresh_expires_at TIMESTAMPTZ NOT NULL,
	replaced_at TIMESTAMPTZ,
	created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX agent_tokens_connection_id ON agent_tokens (connection_id);
CREATE INDEX agent_tokens_refresh_expires_at ON agent_tokens (refresh_expires_at);
