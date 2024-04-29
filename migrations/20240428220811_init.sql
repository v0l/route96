create table users
(
    id      integer unsigned not null auto_increment primary key,
    pubkey  binary(32) not null,
    created timestamp default current_timestamp
);
create unique index ix_user_pubkey on users (pubkey);
create table uploads
(
    id      binary(32) not null primary key,
    user_id integer unsigned not null,
    name    varchar(256) not null,
    size    integer unsigned not null,
    created timestamp default current_timestamp,

    constraint fk_uploads_user
        foreign key (user_id) references users (id)
            on delete cascade
            on update restrict
);