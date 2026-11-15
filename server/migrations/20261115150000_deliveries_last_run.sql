-- A schedule's latest run, read before recording a skipped one so a streak of skips shows once
CREATE INDEX deliveries_last_run ON deliveries (scheduled_message_id, created_at DESC, id DESC);
DROP INDEX deliveries_scheduled_message_id;
