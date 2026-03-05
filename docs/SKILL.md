---
name: nostr-download
description: Upload, download, delete, and list files on a Route96 blob storage server using Blossom and NIP-96 protocols. Use when the user wants to store or retrieve files on a Nostr-integrated CDN, mirror files between servers, or manage uploaded media.
metadata:
  author: v0l
  version: "1.0"
compatibility: Requires nak CLI tool for NIP-98 authenticated requests. Blossom auth (kind 24242) must be constructed manually.
---

# Route96 Blob Storage API

Route96 is a decentralized blob storage server supporting the **Blossom** and **NIP-96** protocols.

**Base URL:** `https://nostr.download` (or set via the `ROUTE96_URL` environment variable).

## Authentication

There are two auth schemes depending on the protocol:

### NIP-98 (NIP-96 endpoints + admin)

NIP-98 uses a kind `27235` Nostr event with `u` (URL) and `method` tags. `nak curl` handles this automatically:

```bash
NOSTR_SECRET_KEY=$(cat ~/.nostr/key.nsec) nak curl [curl options] <url>
```

**Important:** Do NOT use the `--sec` flag — it does not work with `nak curl`. Always set `NOSTR_SECRET_KEY` inline.

### Blossom auth (Blossom endpoints)

Blossom uses a kind `24242` Nostr event with a `t` tag for the action type and an `expiration` tag. `nak curl` does **not** handle this — the auth event must be constructed manually:

```bash
# 1. Build the auth event
AUTH_EVENT=$(nak event \
  --kind 24242 \
  -t t=upload \
  -t expiration=$(($(date +%s) + 300)) \
  --sec $(cat ~/.nostr/key.nsec))

# 2. Base64-encode it
AUTH_HEADER="Nostr $(echo "$AUTH_EVENT" | base64 -w0)"

# 3. Use with curl
curl -X PUT \
  -H "Authorization: $AUTH_HEADER" \
  -H "Content-Type: image/jpeg" \
  --data-binary @photo.jpg \
  "$ROUTE96_URL/upload"
```

Action types for the `t` tag: `upload`, `delete`, `list`.

## NIP-96 Endpoints

These use NIP-98 auth — `nak curl` works directly.

### Upload a file

```bash
NOSTR_SECRET_KEY=$(cat ~/.nostr/key.nsec) nak curl -X POST \
  -F "file=@photo.jpg" \
  -F "content_type=image/jpeg" \
  "$ROUTE96_URL/n96"
```

Optional form fields: `expiration`, `size`, `alt`, `caption`, `content_type`, `no_transform` (set to `true` to skip compression).

Response:

```json
{
  "status": "success",
  "nip94_event": {
    "created_at": 1704067200,
    "content": "photo.jpg",
    "tags": [
      ["url", "https://nostr.download/abc123.jpg"],
      ["ox", "abc123..."],
      ["x", "abc123..."],
      ["m", "image/jpeg"],
      ["size", "204800"],
      ["dim", "1920x1080"]
    ]
  }
}
```

### Delete a file

```bash
NOSTR_SECRET_KEY=$(cat ~/.nostr/key.nsec) nak curl -X DELETE \
  "$ROUTE96_URL/n96/<sha256>"
```

### List your files

```bash
NOSTR_SECRET_KEY=$(cat ~/.nostr/key.nsec) nak curl "$ROUTE96_URL/n96?page=0&count=50"
```

### Get server info

No auth needed:

```bash
curl "$ROUTE96_URL/.well-known/nostr/nip96.json"
```

## Blossom Endpoints

These use kind `24242` auth — see [Blossom auth](#blossom-auth-blossom-endpoints) above.

### Upload a file

```bash
curl -X PUT \
  -H "Authorization: $AUTH_HEADER" \
  -H "Content-Type: image/jpeg" \
  --data-binary @photo.jpg \
  "$ROUTE96_URL/upload"
```

Response:

```json
{
  "url": "https://nostr.download/abc123.jpg",
  "sha256": "abc123...",
  "size": 204800,
  "type": "image/jpeg",
  "uploaded": 1704067200
}
```

### Upload with media compression

Converts images to WebP (requires `media-compression` on the server):

```bash
curl -X PUT \
  -H "Authorization: $AUTH_HEADER" \
  -H "Content-Type: image/jpeg" \
  --data-binary @photo.jpg \
  "$ROUTE96_URL/media"
```

### Mirror a file from URL

```bash
curl -X PUT \
  -H "Authorization: $AUTH_HEADER" \
  -H "Content-Type: application/json" \
  -d '{"url": "https://other-server.com/abc123.jpg"}' \
  "$ROUTE96_URL/mirror"
```

### Delete a file

```bash
curl -X DELETE \
  -H "Authorization: $AUTH_HEADER" \
  "$ROUTE96_URL/<sha256>"
```

### List files by pubkey

No auth needed:

```bash
curl "$ROUTE96_URL/list/<hex_pubkey>"
```

### Report a file

```bash
curl -X PUT \
  -H "Authorization: $AUTH_HEADER" \
  -H "Content-Type: application/json" \
  -d '{"event": <nostr_report_event_json>}' \
  "$ROUTE96_URL/report"
```

## Public Endpoints (no auth)

### Download a file

```bash
curl -o file.jpg "$ROUTE96_URL/<sha256>"
```

### Check if a file exists

```bash
curl -I "$ROUTE96_URL/<sha256>"
```

### Get a thumbnail

Requires `media-compression` on the server:

```bash
curl -o thumb.webp "$ROUTE96_URL/thumb/<sha256>"
```

## Endpoint Reference

| Action              | Method | Endpoint                          | Auth        |
|---------------------|--------|-----------------------------------|-------------|
| Download file       | GET    | `/{sha256}`                       | None        |
| Check file exists   | HEAD   | `/{sha256}`                       | None        |
| Get thumbnail       | GET    | `/thumb/{sha256}`                 | None        |
| Upload (Blossom)    | PUT    | `/upload`                         | Kind 24242  |
| Upload media        | PUT    | `/media`                          | Kind 24242  |
| Mirror file         | PUT    | `/mirror`                         | Kind 24242  |
| Delete (Blossom)    | DELETE | `/{sha256}`                       | Kind 24242  |
| List by pubkey      | GET    | `/list/{pubkey}`                  | None        |
| Report file         | PUT    | `/report`                         | Kind 24242  |
| Server info         | GET    | `/.well-known/nostr/nip96.json`   | None        |
| Upload (NIP-96)     | POST   | `/n96`                            | NIP-98      |
| Delete (NIP-96)     | DELETE | `/n96/{sha256}`                   | NIP-98      |
| List own (NIP-96)   | GET    | `/n96`                            | NIP-98      |

## Notes

- File IDs are SHA-256 hashes of the file content (hex-encoded)
- Uploads are deduplicated by hash — uploading the same file twice returns the existing entry
- The server may enforce a whitelist of allowed pubkeys
- Maximum upload size is configured server-side (default 100 MiB)
- Thumbnails are generated on first request and cached
