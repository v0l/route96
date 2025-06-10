-- Create reports table for file reporting functionality
create table reports
(
    id          integer unsigned not null auto_increment primary key,
    file_id     binary(32) not null,
    reporter_id integer unsigned not null,
    event_json  text not null,
    created     timestamp default current_timestamp,

    constraint fk_reports_file
        foreign key (file_id) references uploads (id)
            on delete cascade
            on update restrict,
    
    constraint fk_reports_reporter
        foreign key (reporter_id) references users (id)
            on delete cascade
            on update restrict
);

-- Unique index to prevent duplicate reports from same user for same file
create unique index ix_reports_file_reporter on reports (file_id, reporter_id);

-- Index for efficient lookups by file
create index ix_reports_file_id on reports (file_id);

-- Index for efficient lookups by reporter
create index ix_reports_reporter_id on reports (reporter_id);