-- Add reviewed flag to reports table
alter table reports add column reviewed boolean not null default false;

-- Index for efficient filtering of non-reviewed reports
create index ix_reports_reviewed on reports (reviewed);