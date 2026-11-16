-- Messages of the chats some agent connection may read, kept for a short window. Which chat, when
-- and whether the owner sent it stay in the clear; who wrote it and what it says are sealed.
CREATE TABLE inbox_messages (
	account_id UUID NOT NULL REFERENCES whatsapp_accounts (id) ON DELETE CASCADE,
	chat_type TEXT NOT NULL CHECK (chat_type IN ('contact', 'group')),
	-- As a grant names a chat: digits for a contact, the group's id for a group
	chat_id TEXT NOT NULL,
	message_id TEXT NOT NULL,
	from_me BOOLEAN NOT NULL,
	sent_at TIMESTAMPTZ NOT NULL,
	-- Set by a disappearing chat's timer, when that ends before the window does
	expires_at TIMESTAMPTZ,
	body BYTEA NOT NULL,
	PRIMARY KEY (account_id, chat_type, chat_id, message_id)
);

CREATE INDEX inbox_messages_recent ON inbox_messages (account_id, chat_type, chat_id, sent_at DESC);
CREATE INDEX inbox_messages_sent_at ON inbox_messages (sent_at);
