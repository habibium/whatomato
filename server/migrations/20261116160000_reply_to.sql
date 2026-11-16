-- The id of the message a send quotes, for an agent's reply
ALTER TABLE deliveries ADD COLUMN reply_to TEXT;
