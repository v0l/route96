-- Add migration script here
alter table uploads
    drop constraint fk_uploads_user;
create table user_uploads
(
    file    BYTEA not null,
    user_id bigint not null,
    created timestamptz default current_timestamp,

    primary key (file, user_id),
    constraint fk_user_uploads_file
        foreign key (file) references uploads (id)
            on delete cascade
            on update restrict,
    constraint fk_user_uploads_user
        foreign key (user_id) references users (id)
            on delete cascade
            on update restrict
);

insert into user_uploads(file, user_id, created)
select uploads.id, uploads.user_id, uploads.created
from uploads;

alter table uploads
    drop column user_id;