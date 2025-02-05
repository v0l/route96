-- Add migration script here
alter table users
    add column paid_until timestamp,
    add column paid_size integer unsigned not null;

create table payments
(
    payment_hash binary(32) not null primary key,
    user_id      integer unsigned not null,
    created      timestamp default current_timestamp,
    amount       integer unsigned not null,
    is_paid      bit(1) not null default 0,
    days_value   integer unsigned not null,
    size_value   integer unsigned not null,
    settle_index integer unsigned,
    rate         float,

    constraint fk_payments_user_id
        foreign key (user_id) references users (id)
            on delete cascade
            on update restrict
);