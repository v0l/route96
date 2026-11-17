create table if not exists user_usage
(
    user_id       integer unsigned not null,
    file_count    bigint unsigned  not null default 0,
    total_size    bigint unsigned  not null default 0,
    egress_bytes  bigint unsigned  not null default 0,
    last_upload   timestamp        null,
    primary key (user_id),
    index ix_user_usage_total_size (total_size),
    index ix_user_usage_file_count (file_count),
    index ix_user_usage_egress (egress_bytes),
    index ix_user_usage_last_upload (last_upload),
    constraint fk_user_usage_user foreign key (user_id) references users (id) on delete cascade
);

insert into user_usage (user_id, file_count, total_size, egress_bytes, last_upload)
select uu.user_id,
       count(*),
       cast(sum(up.size) as unsigned),
       cast(sum(coalesce(fs.egress_bytes, 0)) as unsigned),
       max(uu.created)
from user_uploads uu
         join uploads up on up.id = uu.file
         left join file_stats fs on fs.file = uu.file
group by uu.user_id;
