-- Add migration script here
alter table uploads
    add column duration float,
    add column bitrate integer;