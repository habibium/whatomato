-- WhatsApp logs out a linked device it hasn't seen for a while, so a paused account connects for a
-- moment at this time
ALTER TABLE whatsapp_accounts ADD COLUMN keepalive_at TIMESTAMPTZ;

UPDATE whatsapp_accounts SET keepalive_at = paused_at + INTERVAL '7 days' WHERE paused_at IS NOT NULL;
