-- Add migration script here
alter table users
    add column paid_until timestamp,
    add column paid_space integer unsigned not null;

create table payments
(
    payment_hash binary(32) not null primary key,
    user_id      integer unsigned not null,
    created      timestamp default current_timestamp,
    amount       integer unsigned not null,
    is_paid      bit(1) not null,
    days_value   integer unsigned not null,
    size_value   integer unsigned not null,
    index        integer unsigned not null,
    rate         double not null,

    constraint fk_payments_user
        foreign key (user_id) references users (id)
            on delete cascade
            on update restrict
);