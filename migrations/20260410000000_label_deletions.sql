create table label_deletions
(
    id         bigint unsigned not null auto_increment primary key,
    file       binary(32)      not null,
    pubkeys    text            not null,
    term       varchar(255)    not null,
    label      varchar(255)    not null,
    model      varchar(255)    not null,
    confidence float           not null,
    reason     varchar(1024)   not null,
    dry_run    boolean         not null,
    created    timestamp       not null default current_timestamp
);
create index ix_label_deletions_file on label_deletions (file);
create index ix_label_deletions_dry_run on label_deletions (dry_run, created);
