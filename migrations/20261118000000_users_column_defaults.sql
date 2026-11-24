alter table users
    modify column is_admin bit(1) not null default 0,
    modify column paid_size bigint unsigned not null default 0;
