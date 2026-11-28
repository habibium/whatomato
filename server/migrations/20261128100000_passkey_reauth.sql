-- A passkey asked for again before a sensitive change, like the password; only a sign-in starts
-- before anyone is known
ALTER TABLE passkey_ceremonies
	DROP CONSTRAINT passkey_ceremonies_kind_check,
	DROP CONSTRAINT passkey_ceremonies_check,
	ADD CONSTRAINT passkey_ceremonies_kind_check
		CHECK (kind IN ('registration', 'authentication', 'reauthentication')),
	ADD CONSTRAINT passkey_ceremonies_check CHECK ((kind = 'authentication') = (user_id IS NULL));
