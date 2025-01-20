-- Add migration script here
create table reports
(
    file_id         binary(32)        not null,
    report_id       binary(32)        not null,
    reporter_pubkey binary(32) not null,
    event           varchar(8192) not null,
    received        timestamp default current_timestamp,

    constraint fk_reports_file_id
        foreign key (file_id) references uploads (id)
            on delete cascade
            on update restrict
);
create unique index ix_reports_file_report_id on reports (file_id, report_id);