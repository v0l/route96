# Audit PRs #102 to #109: what landed (2026-11)

Follow-up to `docs/review-2026-11-pr-audit.md` (branch `issue-318-review-the-eight-open-audit-prs-102-to-1`). Seven PRs and the #329 and #330 fixes are on `main`, each pushed as a separate fast-forward. Nothing was posted to GitHub and nothing was deployed.

Every step was checked with `cargo test --lib -- --include-ignored` against a local MariaDB (216 tests at the end), `cargo clippy --all-targets` (no new warnings in touched files), `rustfmt --check` on the lines each change added, and `cargo check --no-default-features --lib --bins --tests` for every feature set in `.github/workflows/features.yml`.

| PR | State | Merge commit |
|---|---|---|
| #107 pin candle fork | merged | `5e21b69` |
| #106 verify auth first, log hygiene | merged | `7fd3f9b` |
| #108 same-origin CORS on admin paths | merged | `b19dd9b` |
| #109 ffmpeg playlist gating | merged | `a0c2ea9` |
| #102 blob response hardening | merged | `3e489a6` |
| #103 BUD-11 list verb | merged | `7b5be14` |
| #105 EXIF/stego in shared ingest | merged | `75c6546` |
| #104 body limit scoping | **not merged, still open** | none |
| issue-330 NIP-96 body limit | merged | `488253c` |
| issue-329 sha256.ext filenames | merged | `f491ea3` |

## #107

Merged as is. Only conflict was `Cargo.toml` next to #328's ffmpeg-rs-raw rev pin; both kept, `cargo metadata --locked` passes.

**Production note:** the branch `fix/disable-bf16-wmma-pre-ampere` on `DrJesseGlass/candle` is deleted upstream. The pinned commit `bb4d702` is reachable only through `refs/pull/13/head` on that fork and can disappear. Mirror it into a v0l-owned fork.

## #106

- Conflict in `src/auth/nip98.rs` with #327. Resolved so `verify()` runs right after the kind check, before #327's `created_at` window, expiration and `u` tag host checks. #106's own default-expiration block was dropped, #327's window replaces it.
- rustfmt on both new `debug!` calls.
- Added `extractor_checks_signature_before_timestamp`: a tampered event that is also a day old gets `Event signature invalid`.

**Production note:** nostr.download runs `RUST_LOG=info`, so the pod logs (and any log shipper) already hold full kind-24242 Blossom auth events, which are replayable until their `expiration`. Purging or rotating those logs is the user's call.

## #108

- Conflict in `src/bin/main.rs` with #331's `lnd` tuple; kept #331's `lnd.as_ref().map(...)` with #108's hoisted `app_state`.
- Imports in `src/cors.rs` (`Request`, `Next`, `Response`, `IntoResponse`, `AppState`) and `use axum::middleware` in `main.rs` are no longer gated on `blossom`. All six feature sets in the CI matrix now build, including `""` and `nip96,react-ui`.
- `cors.rs` formatted.

Unverified: local UI development against a remote server (Vite on `localhost:5173`) gets 403 on `/admin`, `/user`, `/setup` because the Origin matches neither Host nor `public_url`. Not changed.

## #109

- Stray `.gitignore` edit reverted; `ui_src/*.tsbuildinfo` is ignored again.
- `src/phash.rs::decode_rgb` now runs `check_demuxer_format` after the probe. Test `test_phash_image_rejects_playlist_declared_as_image`.
- Every #109 site now opens through #328's `open_demuxer` / `Transcoder::new_with_open_options`, so the format and protocol whitelist applies at open time.
- **Dropped as redundant:** `reject_playlist_media` (the `#EXTM3U` sniff before demuxing) and its two tests. #328's `format_whitelist` does not include `hls`, so an HLS playlist fails to open before any probe, and `hls_playlist_fails_to_open` in `src/processing/mod.rs` covers it. The sniff caught only HLS anyway.
- **Kept as defence in depth:** `check_demuxer_format` after every probe (`hls, applehttp, concat, dash, rtsp, sdp`).
- Stale doc comment that said ffmpeg-rs-raw passes no options dictionary was corrected.

## #102

- clippy `needless_borrow` fixed (`set_file_headers(&mut response, info)`).
- `application/pdf` is not in the inert list (it never was), and PDFs now get **no** `Content-Security-Policy: sandbox`, because the sandbox CSP stops Chromium's PDF viewer. They keep `nosniff` and their declared type. Every other blob keeps the sandbox CSP. Tests: `pdf_blobs_skip_sandbox_csp_so_the_viewer_renders_inline`, `octet_stream_blobs_keep_type_and_sandbox`.
- CORS on blob GETs is unchanged. DB-backed test `cross_origin_blob_get_keeps_wildcard_cors_and_hardening_headers` stores a PDF, an `application/octet-stream` blob and an HTML blob, then does cross-origin GET and preflight through `cors_layer` and checks `Access-Control-Allow-Origin: *`, `nosniff`, the served type and the CSP on each.

Unverified (needs a real desktop browser, the headless one renders no PDF either way):

- A PDF blob opens inline in desktop Chrome and Firefox with the headers above.
- A blob stored as `application/octet-stream` and embedded cross-origin in `<img>`/`<video>` is not blocked by Chromium's ORB now that `nosniff` is sent.
- Uploaded PDFs are no longer sandboxed. PDF JavaScript runs inside the viewer, not on the origin, but this relies on the browser's viewer.

## #103

- clippy `unnecessary_to_owned` in its test fixed (`hex::encode(self_auth.event.pubkey.to_bytes())`).
- No conflict, since #104 was not merged.

## #105

- The 25 MiB stego cap no longer skips the scan. With `reject_steganography` on, an image whose stored file is over the cap is **rejected** (422 on Blossom and NIP-96, removed from disk).
- The cap is a setting, `steganography_scan_max_bytes`, default `26214400` (25 MiB), documented in `src/settings.rs`, `config.yaml` and the admin config editor.
- Tests: `jpeg_padded_past_stego_cap_is_rejected_not_stored` (a real JPEG padded with 26 MiB after EOI, declared both `image/jpeg` and `application/octet-stream`; nothing left in storage) and `stego_cap_setting_is_honoured`. Both fail with the reject branch disabled.
- One #105 rustfmt diff in `filesystem.rs` fixed.

## #104

Not merged and left untouched, as the review concluded. `/report` and `/mirror` were already capped at 2 MiB on `main`, and the real body-limit bug is fixed by the issue-330 merge. **The user needs to close PR #104 on GitHub** (closing it posts to the author, so it was not done here).

## issue-330 (NIP-96 uploads over 2 MiB)

- `DefaultBodyLimit::disable()` on `/n96`; the no-op global `RequestBodyLimitLayer` removed from `main.rs` (resolved next to #108's layer).
- Its test was adapted to #327: the `u` tag is now `https://example.com/n96` with a matching Host. Before that it got 401 and passed for the wrong reason. Confirmed it fails with the fix removed.

## issue-329 (sha256.ext filenames)

- Merged cleanly. Added on top: the extension comes from the served type, so an HTML blob rewritten to `text/plain` by #102 is named `<sha256>.txt`, not `.html`. Test `rewritten_blobs_get_the_served_type_extension`.
