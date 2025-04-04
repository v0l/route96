-- Add migration script here
-- Set default value for is_admin column for new rows
ALTER TABLE users
    ALTER COLUMN is_admin SET DEFAULT false;