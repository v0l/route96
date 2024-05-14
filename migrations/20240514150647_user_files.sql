-- Add migration script here
alter table uploads
    drop constraint fk_uploads_user;
create table user_uploads
(
    file    binary(32)       not null,
    user_id integer unsigned not null,
    created timestamp default current_timestamp,

    constraint fk_user_uploads_file_id
        foreign key (file) references uploads (id)
            on delete cascade
            on update restrict,
    constraint fk_user_uploads_user_id
        foreign key (user_id) references users (id)
            on delete cascade
            on update restrict
);
create unique index ix_user_uploads_file_pubkey on user_uploads (file, user_id);

insert into user_uploads(file, user_id, created)
select uploads.id, uploads.user_id, uploads.created
from uploads;

alter table uploads
    drop column user_id;