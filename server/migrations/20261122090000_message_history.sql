-- The owner's choice to keep every chat's messages, not only those of chats a connection may
-- read. Without a row the inbox keeps readable chats for 7 days. A NULL `keep_days` keeps
-- everything.
CREATE TABLE message_history (
	user_id UUID PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
	keep_days INTEGER CHECK (keep_days > 0),
	updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- History the phone sent, sealed and held from its receipt until its messages are kept, so a
-- restart mid-import loses nothing
CREATE TABLE history_chunks (
	account_id UUID NOT NULL REFERENCES whatsapp_accounts (id) ON DELETE CASCADE,
	message_id TEXT NOT NULL,
	chunk BYTEA NOT NULL,
	received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
	PRIMARY KEY (account_id, message_id)
);

-- How far the import of the phone's history has got, shown to the owner
ALTER TABLE whatsapp_accounts
	ADD COLUMN history_progress INTEGER,
	ADD COLUMN history_imported BIGINT NOT NULL DEFAULT 0;

-- Keyed hashes of each message's three-character slices, so a search finds messages without
-- their text being stored in the clear. NULL until computed.
ALTER TABLE inbox_messages ADD COLUMN terms INTEGER[];
CREATE INDEX inbox_messages_terms ON inbox_messages USING GIN (terms);
CREATE INDEX inbox_messages_unindexed ON inbox_messages (account_id) WHERE terms IS NULL;
