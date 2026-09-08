-- Ban flag for user pubkeys. Banned users are rejected on every authenticated
-- write path (upload, mirror, delete, report).
alter table users
    add column banned     bit(1)       not null default 0,
    add column ban_reason varchar(512) null;
