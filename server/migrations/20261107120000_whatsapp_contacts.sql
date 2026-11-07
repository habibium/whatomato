-- The address book the phone shares with linked devices, so a message can go to a name. Names are
-- sealed; `jid`, `phone_number` and `lid` stay plain because lookups join on them.
CREATE TABLE whatsapp_contacts (
	account_id UUID NOT NULL REFERENCES whatsapp_accounts (id) ON DELETE CASCADE,
	jid TEXT NOT NULL,
	phone_number TEXT,
	lid TEXT,
	names BYTEA NOT NULL,
	updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
	PRIMARY KEY (account_id, jid)
);

-- Set once the whole address book has been replayed; until then, every connect replays it
ALTER TABLE whatsapp_accounts ADD COLUMN contacts_imported_at TIMESTAMPTZ;
