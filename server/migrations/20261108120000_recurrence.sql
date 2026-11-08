-- How a recurring message repeats, as the server's `Recurrence`: a cron expression, the sun's times,
-- and kinds to come. JSON, so a new kind needs no migration.
ALTER TABLE scheduled_messages ADD COLUMN recurrence JSONB;

UPDATE scheduled_messages
SET recurrence = jsonb_build_object('type', 'cron', 'expression', cron)
WHERE cron IS NOT NULL;

ALTER TABLE scheduled_messages DROP CONSTRAINT scheduled_messages_check;
ALTER TABLE scheduled_messages DROP COLUMN cron;

ALTER TABLE scheduled_messages ADD CONSTRAINT scheduled_messages_schedule CHECK (
	(schedule_kind = 'once' AND run_at IS NOT NULL AND recurrence IS NULL)
	OR (schedule_kind = 'recurring' AND recurrence IS NOT NULL AND run_at IS NULL)
);
