-- Add h_tag column to uploads table
ALTER TABLE uploads
ADD COLUMN h_tag VARCHAR(256) NULL;