-- Moderation columns added to uploads table
--
-- review_state: workflow state
--   0 = None         (default, no review needed)
--   1 = LabelFlagged (auto-flagged by AI label match)
--   2 = Reported     (flagged by user report)
--   3 = Reviewed     (admin has reviewed and cleared)
--
-- banned: soft-delete tombstone — row is kept so re-uploads of the same hash
--   are rejected, but the physical file and all ownership records are removed.
--
-- labeled_by: comma-separated list of model names that have already labeled
--   this file, used by the background task to skip re-processing.
alter table uploads
    add column review_state tinyint unsigned not null default 0,
    add column banned       boolean          not null default false,
    add column labeled_by   varchar(512)     not null default '';

create index ix_uploads_review_state on uploads (review_state);
create index ix_uploads_banned       on uploads (banned);
