-- A stop the user has to act on, a takeover or a temporary ban, outlives a restart, so boot doesn't
-- reconnect with keys in use elsewhere or during the ban
ALTER TABLE whatsapp_accounts
	ADD COLUMN stopped_reason TEXT CHECK (stopped_reason IN ('replaced', 'temporary_ban')),
	ADD COLUMN stopped_until TIMESTAMPTZ,
	ADD CHECK (
		CASE stopped_reason
			WHEN 'temporary_ban' THEN stopped_until IS NOT NULL
			ELSE stopped_until IS NULL
		END
	);
