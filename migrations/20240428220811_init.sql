create table users
(
    id      BIGSERIAL primary key,
    pubkey  BYTEA not null,
    created timestamptz default current_timestamp
);
create unique index ix_user_pubkey on users (pubkey);

create table uploads
(
    id      BYTEA not null primary key,
    user_id bigint not null,
    size    bigint not null,
    mime_type varchar(128) not null,
    created timestamptz default current_timestamp,

    constraint fk_uploads_user
        foreign key (user_id) references users (id)
            on delete cascade
            on update restrict
);