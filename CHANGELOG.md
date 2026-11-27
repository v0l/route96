# Changelog

All notable changes to route96 are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.8.1] - 2026-11-27

### Changed
- The Docker image runs as uid 10001 instead of root (images since 17 Nov). Existing installs must hand their config file and data directory to that uid; see "Upgrading from images before 17 Nov" in the README. `/app/data` in the image is owned by uid 10001, so new named volumes get the right owner.

### Fixed
- Setup on MariaDB in strict mode no longer fails with `Field 'paid_size' doesn't have a default value`: `users.paid_size` and `users.is_admin` now default to 0 (#114).
- Startup permission errors name the path, its owner and mode and the process uid instead of a bare `Permission denied (os error 13)`, and the Docker entrypoint prints the exact `chown` and `chmod` commands before exiting with code 1.
- Lightning payments that settle while route96 is restarting, or while the LND invoice stream is down, are now credited. The LND `settle_index` is stored with each settled payment, the invoice stream resubscribes from it with backoff when it ends or errors, and on start and after each reconnect unpaid invoices from the last `payment_repair_days` days (default 7, editable in the admin config editor) are looked up in LND and the settled ones are credited once. A replayed settle no longer extends a subscription twice (#115).

## [0.8.0] - 2026-11-09

### Security
- NIP-98 auth checks that the `u` tag names this server's own host and that `created_at` is within 60 seconds of now. Signatures are verified before any tag is read, for both NIP-98 and Blossom auth, and full auth events are no longer written to the logs.
- ffmpeg opens uploads with a whitelist of input formats (common video, audio and image containers), so playlist and reference formats such as HLS, concat and DASH cannot make it read local files. The demuxer format is checked again after every probe.
- Blobs are served with `X-Content-Type-Options: nosniff` and a sandboxing `Content-Security-Policy` (PDFs excepted, so the browser viewer works). HTML, XHTML, XML and JavaScript are served as `text/plain`.
- `/admin`, `/user` and `/setup` answer CORS for the same origin only.
- EXIF and steganography checks run for every upload protocol, NIP-96 included, on the sniffed type rather than the declared one. With `reject_steganography` on, an image larger than `steganography_scan_max_bytes` (default 25 MiB) is rejected.
- `/list` requires the `list` verb when auth is given, and listings of another user's files leave out their NIP-94 metadata.
- `/mirror` refuses private, loopback, link-local and metadata addresses, pins the address it checked so DNS rebinding cannot get round it, and has timeouts.
- NIP-96 uploads are streamed to disk with a hard size cap. Upload quotas are always enforced, banned files are never served or accepted again, whitelist lookup errors fail closed, and a malformed auth expiration tag or a mirrored `Content-Type` no longer panics the server.
- The candle dependency is pinned to a fixed commit on the v0l fork.

### Added
- Hot and cold tiered storage: files that are old and rarely downloaded move to `cold_storage_dir` and come back when they get popular. The admin stats show the size of each tier.
- BUD-13 path uploads, Blossom Base64URL hashes, `X-SHA-256` on uploads and the BUD-09 report endpoint. The blossom compliance suite runs in CI.
- Admins can ban a pubkey.
- Bulk actions in the admin file list, sorting by size and download count, and a progress view for background processing.
- A review GUI with label overrides, plus scripts to fine-tune and compare NSFW models. The default NSFW model is `route96-nsfw-vit224`.
- A CI matrix that builds every supported feature combination.

### Changed
- Blobs are served with the filename `<sha256>.<ext>`, the extension taken from the type they are served as, instead of the uploader's filename.
- Faster GETs: small blob bodies and blob lookups are cached, TCP nodelay is on, and uploads are hashed while they stream.
- Errors use the right HTTP status codes and set an `X-Reason` header on auth rejections.
- Media smaller than 48 px on either side is not labelled, and labelling tasks time out after 5 minutes.

### Fixed
- NIP-96 uploads over 2 MiB work again.
- Builds with any feature combination compile, including builds without `media-compression` or `labels`.
- The payments handler finds the last settled invoice correctly at startup.
- Mirroring follows redirects.
- Range requests work for every file size, an unsatisfiable range gets 416 and a range the server cannot parse is ignored.
- Admin pagination, the downloads sort, report and `created` column decoding on MariaDB, and first admin setup when the user row already exists.
- Image compression and ffmpeg packet loops can no longer spin the CPU on corrupt files.
- The file picker keeps slow selections.
