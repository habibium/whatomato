-- A picture or file an agent sends, sealed and held only until it goes out: its data is dropped
-- once the send is over, or once nothing waits for it any more
CREATE TABLE outgoing_files (
	id UUID PRIMARY KEY,
	user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
	kind TEXT NOT NULL CHECK (kind IN ('image', 'video', 'audio', 'document')),
	mime_type TEXT NOT NULL,
	file_name TEXT,
	size INTEGER NOT NULL,
	data BYTEA,
	created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX outgoing_files_held ON outgoing_files (created_at) WHERE data IS NOT NULL;

ALTER TABLE deliveries ADD COLUMN file_id UUID REFERENCES outgoing_files (id) ON DELETE SET NULL;
CREATE INDEX deliveries_file_id ON deliveries (file_id) WHERE file_id IS NOT NULL;
