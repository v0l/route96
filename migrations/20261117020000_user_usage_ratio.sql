alter table user_usage
    add column traffic_ratio double as (egress_bytes / nullif(total_size, 0)) persistent,
    add index ix_user_usage_traffic_ratio (traffic_ratio);
