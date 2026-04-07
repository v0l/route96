-- Add index on created column for faster date range queries
CREATE INDEX idx_uploads_created ON uploads(created);
