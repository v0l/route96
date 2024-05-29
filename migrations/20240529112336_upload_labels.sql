-- Add migration script here
alter table uploads
    add column blur_hash varchar(512),
    add column width integer unsigned,
    add column height integer unsigned;

create table upload_labels
(
    file    binary(32)        not null,
    label   varchar(255)      not null,
    created timestamp default current_timestamp,
    model   varchar(255)      not null,

    constraint fk_upload_labels_file_id
        foreign key (file) references uploads (id)
            on delete cascade
            on update restrict
);
create unique index ix_upload_labels_file_label_model on upload_labels (file, label, model);