-- Composite index for paginated queries: WHERE banned = false ORDER BY created
-- Replaces the standalone ix_uploads_banned index for most query patterns.
create index ix_uploads_banned_created on uploads (banned, created);
drop index ix_uploads_banned on uploads;

-- Index on mime_type for LIKE prefix filtering and label worker queries
create index ix_uploads_mime_type on uploads (mime_type);

-- Standalone index on user_uploads.user_id for per-user joins
-- (the existing unique index is (file, user_id) so user_id-first lookups can't use it)
create index ix_user_uploads_user_id on user_uploads (user_id);

-- Payments: index for per-user payment lookups
create index ix_payments_user_id on payments (user_id);

-- Payments: composite index for settle_index queries (subscribe from last settled)
create index ix_payments_is_paid_settle on payments (is_paid, settle_index);
