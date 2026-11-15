-- A run of an agent's schedule that its connection could no longer make: recorded so the owner
-- sees it in History, never attempted, and never counted against the connection's limits
ALTER TABLE deliveries
	ADD COLUMN skipped BOOLEAN NOT NULL DEFAULT false,
	ADD CONSTRAINT deliveries_skipped_failed CHECK (NOT skipped OR status = 'failed');
