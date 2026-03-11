-- Database-backed whitelist table.
-- Each row represents a pubkey (hex) that is allowed to upload files.
-- When whitelist enforcement is enabled (via settings or presence of rows),
-- only pubkeys in this table (or the config/file whitelist) may upload.
create table if not exists whitelist
(
    pubkey  varchar(64)  not null primary key,
    created timestamp    not null default current_timestamp
);
