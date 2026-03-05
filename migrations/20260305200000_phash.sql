-- Perceptual hash (pHash) for image similarity search via LSH.
-- The 64-bit hash is stored as four indexed 16-bit bands.
-- Candidate lookup matches any shared band; exact Hamming distance is
-- verified in application code by XOR-ing the bands.

create table if not exists upload_phash (
    file    binary(32)  not null primary key,
    band0   smallint    not null,
    band1   smallint    not null,
    band2   smallint    not null,
    band3   smallint    not null,
    created datetime    not null default current_timestamp,
    constraint fk_phash_file foreign key (file) references uploads(id) on delete cascade
);

create index idx_phash_band0 on upload_phash(band0);
create index idx_phash_band1 on upload_phash(band1);
create index idx_phash_band2 on upload_phash(band2);
create index idx_phash_band3 on upload_phash(band3);
