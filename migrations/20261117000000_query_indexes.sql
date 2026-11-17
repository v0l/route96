alter table users
    add index ix_users_banned_created (banned, created),
    add index ix_users_paid_until (paid_until);

alter table uploads
    add index ix_uploads_banned_size (banned, size);
