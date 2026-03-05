-- Replace the comma-separated `labeled_by` column with a proper join table
-- so that "files missing labels for model X" can be answered with an indexed
-- NOT EXISTS subquery instead of a full table scan with find_in_set().

create table upload_labeled_by
(
    file  binary(32)   not null,
    model varchar(255) not null,
    created timestamp default current_timestamp,

    primary key (file, model),
    constraint fk_upload_labeled_by_file
        foreign key (file) references uploads (id)
            on delete cascade
            on update restrict
);

alter table uploads drop column labeled_by;
