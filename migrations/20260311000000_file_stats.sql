-- Track per-file access statistics: last access time and cumulative egress bytes.
-- This table is written by the in-memory stats tracker that flushes periodically.
create table if not exists file_stats (
    file       binary(32)          not null,
    last_accessed timestamp         null,
    egress_bytes  bigint unsigned   not null default 0,
    primary key (file),
    constraint fk_file_stats_file foreign key (file) references uploads(id) on delete cascade
);
