-- Keys the instance makes for itself at first start, sealed with `ENCRYPTION_KEY`. `vapid` signs
-- what is sent to browsers' push services.
CREATE TABLE instance_keys (
	name TEXT PRIMARY KEY,
	sealed BYTEA NOT NULL,
	created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- A browser that asked for notifications. `endpoint` is its push service's address for it, and
-- the two keys encrypt each notification so that only that browser can read it.
CREATE TABLE push_subscriptions (
	id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
	user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
	endpoint TEXT NOT NULL UNIQUE CHECK (char_length(endpoint) <= 2048),
	p256dh BYTEA NOT NULL CHECK (octet_length(p256dh) = 65),
	auth BYTEA NOT NULL CHECK (octet_length(auth) = 16),
	label TEXT NOT NULL CHECK (char_length(label) <= 60),
	created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX push_subscriptions_user_id ON push_subscriptions (user_id, created_at);
