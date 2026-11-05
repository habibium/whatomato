-- A user's linked WhatsApp device. `device` is whatsapp-rust's device state, sealed by
-- `crypto::Cipher`, and empty until the library creates its keys. Boot reconnects accounts with a
-- `linked_at` and no `paused_at`.
CREATE TABLE whatsapp_accounts (
	id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
	user_id UUID NOT NULL UNIQUE REFERENCES users (id) ON DELETE CASCADE,
	device BYTEA,
	phone_number TEXT,
	linked_at TIMESTAMPTZ,
	paused_at TIMESTAMPTZ,
	created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
	updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The tables below back whatsapp-rust's storage traits, one account per `account_id`. Values that
-- are secret or private are sealed; keys, timestamps and flags stay plain because lookups and
-- expiry sweeps filter on them. Timestamps are Unix seconds, as the library passes them.

CREATE TABLE whatsapp_identities (
	account_id UUID NOT NULL REFERENCES whatsapp_accounts (id) ON DELETE CASCADE,
	address TEXT NOT NULL,
	key BYTEA NOT NULL,
	PRIMARY KEY (account_id, address)
);

CREATE TABLE whatsapp_sessions (
	account_id UUID NOT NULL REFERENCES whatsapp_accounts (id) ON DELETE CASCADE,
	address TEXT NOT NULL,
	record BYTEA NOT NULL,
	PRIMARY KEY (account_id, address)
);

CREATE TABLE whatsapp_prekeys (
	account_id UUID NOT NULL REFERENCES whatsapp_accounts (id) ON DELETE CASCADE,
	id INTEGER NOT NULL,
	record BYTEA NOT NULL,
	uploaded BOOLEAN NOT NULL,
	PRIMARY KEY (account_id, id)
);

CREATE TABLE whatsapp_signed_prekeys (
	account_id UUID NOT NULL REFERENCES whatsapp_accounts (id) ON DELETE CASCADE,
	id INTEGER NOT NULL,
	record BYTEA NOT NULL,
	PRIMARY KEY (account_id, id)
);

CREATE TABLE whatsapp_sender_keys (
	account_id UUID NOT NULL REFERENCES whatsapp_accounts (id) ON DELETE CASCADE,
	address TEXT NOT NULL,
	record BYTEA NOT NULL,
	PRIMARY KEY (account_id, address)
);

-- Which group members' devices already hold our sender key
CREATE TABLE whatsapp_sender_key_devices (
	account_id UUID NOT NULL REFERENCES whatsapp_accounts (id) ON DELETE CASCADE,
	group_jid TEXT NOT NULL,
	device_jid TEXT NOT NULL,
	has_key BOOLEAN NOT NULL,
	PRIMARY KEY (account_id, group_jid, device_jid)
);

CREATE INDEX whatsapp_sender_key_devices_device_jid_idx ON whatsapp_sender_key_devices (account_id, device_jid);

CREATE SEQUENCE whatsapp_app_state_keys_stored_seq;

-- `stored` grows with every write, so the latest key is the one stored last
CREATE TABLE whatsapp_app_state_keys (
	account_id UUID NOT NULL REFERENCES whatsapp_accounts (id) ON DELETE CASCADE,
	key_id BYTEA NOT NULL,
	key BYTEA NOT NULL,
	stored BIGINT NOT NULL DEFAULT nextval('whatsapp_app_state_keys_stored_seq'),
	PRIMARY KEY (account_id, key_id)
);

CREATE TABLE whatsapp_app_state_versions (
	account_id UUID NOT NULL REFERENCES whatsapp_accounts (id) ON DELETE CASCADE,
	name TEXT NOT NULL,
	state BYTEA NOT NULL,
	PRIMARY KEY (account_id, name)
);

CREATE TABLE whatsapp_app_state_mutation_macs (
	account_id UUID NOT NULL REFERENCES whatsapp_accounts (id) ON DELETE CASCADE,
	name TEXT NOT NULL,
	index_mac BYTEA NOT NULL,
	value_mac BYTEA NOT NULL,
	version BIGINT NOT NULL,
	PRIMARY KEY (account_id, name, index_mac)
);

CREATE TABLE whatsapp_lid_mappings (
	account_id UUID NOT NULL REFERENCES whatsapp_accounts (id) ON DELETE CASCADE,
	lid TEXT NOT NULL,
	phone_number TEXT NOT NULL,
	learning_source TEXT NOT NULL,
	created_at BIGINT NOT NULL,
	updated_at BIGINT NOT NULL,
	PRIMARY KEY (account_id, lid)
);

CREATE INDEX whatsapp_lid_mappings_phone_number_idx ON whatsapp_lid_mappings (account_id, phone_number, updated_at);

CREATE TABLE whatsapp_device_lists (
	account_id UUID NOT NULL REFERENCES whatsapp_accounts (id) ON DELETE CASCADE,
	user_jid TEXT NOT NULL,
	record BYTEA NOT NULL,
	PRIMARY KEY (account_id, user_jid)
);

CREATE TABLE whatsapp_base_keys (
	account_id UUID NOT NULL REFERENCES whatsapp_accounts (id) ON DELETE CASCADE,
	address TEXT NOT NULL,
	message_id TEXT NOT NULL,
	base_key BYTEA NOT NULL,
	created_at BIGINT NOT NULL,
	PRIMARY KEY (account_id, address, message_id)
);

CREATE INDEX whatsapp_base_keys_created_at_idx ON whatsapp_base_keys (account_id, created_at);

-- Privacy tokens for messaging a contact. A NULL `token` is a placeholder that only carries
-- `sender_timestamp`.
CREATE TABLE whatsapp_tc_tokens (
	account_id UUID NOT NULL REFERENCES whatsapp_accounts (id) ON DELETE CASCADE,
	jid TEXT NOT NULL,
	token BYTEA,
	token_timestamp BIGINT NOT NULL,
	sender_timestamp BIGINT,
	PRIMARY KEY (account_id, jid)
);

-- Messages we sent, kept for two hours so the library can answer retry requests
CREATE TABLE whatsapp_sent_messages (
	account_id UUID NOT NULL REFERENCES whatsapp_accounts (id) ON DELETE CASCADE,
	chat_jid TEXT NOT NULL,
	message_id TEXT NOT NULL,
	payload BYTEA NOT NULL,
	created_at BIGINT NOT NULL,
	PRIMARY KEY (account_id, chat_jid, message_id)
);

CREATE INDEX whatsapp_sent_messages_created_at_idx ON whatsapp_sent_messages (account_id, created_at);

-- An `expires_at` of 0 never expires
CREATE TABLE whatsapp_msg_secrets (
	account_id UUID NOT NULL REFERENCES whatsapp_accounts (id) ON DELETE CASCADE,
	chat TEXT NOT NULL,
	sender TEXT NOT NULL,
	msg_id TEXT NOT NULL,
	secret BYTEA NOT NULL,
	expires_at BIGINT NOT NULL,
	message_ts BIGINT NOT NULL,
	PRIMARY KEY (account_id, chat, sender, msg_id)
);

CREATE INDEX whatsapp_msg_secrets_expires_at_idx ON whatsapp_msg_secrets (account_id, expires_at) WHERE expires_at <> 0;
