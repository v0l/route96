-- Add migration script here
alter table uploads
    add column blur_hash varchar(512),
    add column width integer,
    add column height integer;

create table upload_labels
(
    file    BYTEA not null,
    label   varchar(256) not null,
    model   varchar(128) not null,
    created timestamptz default current_timestamp,

    primary key (file, label),
    constraint fk_upload_labels_file
        foreign key (file) references uploads (id)
            on delete cascade
            on update restrict
);