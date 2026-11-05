-- Reusable message bodies.
CREATE TABLE message_templates (
	id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
	user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
	name TEXT NOT NULL,
	body TEXT NOT NULL,
	created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
	updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX message_templates_user_id ON message_templates (user_id);

-- A message sent once at `run_at`, or on every match of `cron` in `timezone`. `next_run_at` is empty
-- while disabled and once a one-time message has gone out.
CREATE TABLE scheduled_messages (
	id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
	user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
	recipient_type TEXT NOT NULL CHECK (recipient_type IN ('contact', 'group')),
	recipient TEXT NOT NULL,
	recipient_name TEXT,
	body TEXT NOT NULL,
	schedule_kind TEXT NOT NULL CHECK (schedule_kind IN ('once', 'recurring')),
	run_at TIMESTAMPTZ,
	cron TEXT,
	timezone TEXT NOT NULL,
	template_id UUID REFERENCES message_templates (id) ON DELETE SET NULL,
	enabled BOOLEAN NOT NULL DEFAULT true,
	next_run_at TIMESTAMPTZ,
	last_run_at TIMESTAMPTZ,
	created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
	updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
	CHECK (
		(schedule_kind = 'once' AND run_at IS NOT NULL AND cron IS NULL)
		OR (schedule_kind = 'recurring' AND cron IS NOT NULL AND run_at IS NULL)
	)
);

CREATE INDEX scheduled_messages_user_id ON scheduled_messages (user_id, created_at);
CREATE INDEX scheduled_messages_due ON scheduled_messages (next_run_at) WHERE enabled;

-- Every send, scheduled or not, and the queue they wait in. A row in `sending` when a server starts
-- may or may not have gone out, so it's failed rather than sent twice.
CREATE TABLE deliveries (
	id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
	user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
	scheduled_message_id UUID REFERENCES scheduled_messages (id) ON DELETE SET NULL,
	recipient_type TEXT NOT NULL CHECK (recipient_type IN ('contact', 'group')),
	recipient TEXT NOT NULL,
	recipient_name TEXT,
	body TEXT NOT NULL,
	status TEXT NOT NULL DEFAULT 'pending'
		CHECK (status IN ('pending', 'sending', 'sent', 'failed')),
	attempts INTEGER NOT NULL DEFAULT 0,
	next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
	error TEXT,
	created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
	sent_at TIMESTAMPTZ
);

CREATE INDEX deliveries_user_id ON deliveries (user_id, created_at DESC);
CREATE INDEX deliveries_scheduled_message_id ON deliveries (scheduled_message_id);
CREATE INDEX deliveries_due ON deliveries (next_attempt_at) WHERE status = 'pending';
