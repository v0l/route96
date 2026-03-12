# Route96 Admin API

All admin endpoints require a valid [NIP-98](https://github.com/nostr-protocol/nips/blob/master/98.md) `Authorization` header signed by a pubkey that has `is_admin = true` in the database.

---

## Self

### `GET /admin/self`

Returns info and storage stats for the authenticated admin user.

**Response**

```json
{
  "status": "success",
  "data": {
    "is_admin": true,
    "file_count": 42,
    "total_size": 104857600
  }
}
```

---

## User Files

### `GET /user/files`

List the authenticated user's own files with full metadata and download statistics. This is the rich alternative to the basic NIP-96 file list.

**Authentication:** [Blossom](https://github.com/hzrd149/blossom) — `Authorization: Nostr <base64>` header containing a signed kind `24242` event with `t: list` and a valid `expiration` tag.

**Query parameters**

| Parameter   | Type   | Default   | Description                                                |
| ----------- | ------ | --------- | ---------------------------------------------------------- |
| `page`      | int    | `0`       | Page number (zero-based)                                   |
| `count`     | int    | `50`      | Results per page (max 5000)                                |
| `mime_type` | string | —         | Filter by MIME type substring                              |
| `label`     | string | —         | Filter to files whose labels contain this substring        |
| `sort`      | string | `created` | Sort column: `created`, `egress_bytes`, or `last_accessed` |
| `order`     | string | `desc`    | Sort direction: `desc` or `asc`                            |

When `sort` is `egress_bytes` or `last_accessed`, only files that have been accessed at least once are included (inner join on `file_stats`).

**Response**

```json
{
  "status": "success",
  "data": {
    "count": 2,
    "page": 0,
    "total": 42,
    "files": [
      {
        "created_at": 1700000000,
        "content": "photo.jpg",
        "tags": [
          ["url", "https://example.com/abc123.jpg"],
          ["x", "abc123..."],
          ["m", "image/jpeg"],
          ["size", "204800"],
          ["thumb", "https://example.com/thumb/abc123.webp"],
          ["blurhash", "LEHV6nWB2yk8pyo0adR*.7kCMdnj"],
          ["dim", "1920x1080"]
        ],
        "stats": {
          "last_accessed": "2026-03-11T12:00:00Z",
          "egress_bytes": 1048576
        }
      }
    ]
  }
}
```

Each file in `files` is a NIP-94 event with an extra `stats` field:

- `stats.egress_bytes` — total bytes served for this file (0 if never downloaded)
- `stats.last_accessed` — ISO 8601 timestamp of the most recent download, or `null`

---

## Files

### `GET /admin/files`

List all files (excluding banned).

**Query parameters**

| Parameter   | Type   | Default   | Description                                                |
| ----------- | ------ | --------- | ---------------------------------------------------------- |
| `page`      | int    | `0`       | Page number (zero-based)                                   |
| `count`     | int    | `50`      | Results per page (max 5000)                                |
| `mime_type` | string | —         | Filter by MIME type substring                              |
| `label`     | string | —         | Filter to files whose labels contain this substring        |
| `sort`      | string | `created` | Sort column: `created`, `egress_bytes`, or `last_accessed` |
| `order`     | string | `desc`    | Sort direction: `desc` or `asc`                            |

When `sort` is `egress_bytes` or `last_accessed`, only files that have been accessed at least once are included (inner join on `file_stats`).

**Response**

```json
{
  "status": "success",
  "data": {
    "count": 10,
    "page": 0,
    "total": 42,
    "files": [
      {
        "url": "https://example.com/abc123...",
        "x": "abc123...",
        "m": "image/jpeg",
        "size": "204800",
        "uploader": ["pubkey_hex"],
        "stats": {
          "last_accessed": "2026-03-11T12:00:00Z",
          "egress_bytes": 1048576
        }
      }
    ]
  }
}
```

Each file in `files` is a NIP-94 event with two extra fields:

- `uploader` — list of hex pubkeys that own the file
- `stats` — access statistics; `last_accessed` is `null` and `egress_bytes` is `0` for files that have never been served

---

### `GET /admin/files/{sha256}/stats`

Return persisted access statistics for a single file.

**Response**

```json
{
  "status": "success",
  "data": {
    "last_accessed": "2026-03-11T12:00:00Z",
    "egress_bytes": 1048576
  }
}
```

`last_accessed` is `null` and `egress_bytes` is `0` when the file has never been served.

---

## Moderation Queue

### `GET /admin/files/review`

List files pending moderation review — those with `review_state` of `LabelFlagged` (1) or `Reported` (2), oldest first.

**Query parameters**

| Parameter | Type | Default | Description                 |
| --------- | ---- | ------- | --------------------------- |
| `page`    | int  | `0`     | Page number (zero-based)    |
| `count`   | int  | `50`    | Results per page (max 5000) |

**Response** — same shape as `GET /admin/files`, including `stats` on each file.

---

### `PATCH /admin/files/review`

Mark one or more files as reviewed, clearing the moderation flag (`review_state → Reviewed`). The files remain accessible.

**Request body**

```json
{ "ids": ["abc123...", "def456..."] }
```

**Response**

```json
{ "status": "success" }
```

---

### `DELETE /admin/files/review`

Permanently ban one or more files. Physical files are removed from disk, all ownership records are deleted, and the database rows are kept as tombstones so the same hashes cannot be re-uploaded.

**Request body**

```json
{ "ids": ["abc123...", "def456..."] }
```

**Response**

```json
{ "status": "success" }
```

---

## Similar Images

### `GET /admin/files/{sha256}/similar`

Find visually similar images using perceptual hashing (pHash + LSH). Requires the `media-compression` feature.

Perceptual hashes are computed at upload time for new images. A background worker also backfills hashes for any images that were uploaded before the feature was enabled. This endpoint finds candidates that share at least one LSH band with the queried file, then verifies exact Hamming distance.

**Query parameters**

| Parameter  | Type | Default | Description                                          |
| ---------- | ---- | ------- | ---------------------------------------------------- |
| `distance` | int  | `10`    | Maximum Hamming distance (0 = exact match, 64 = max) |

**Response**

```json
{
  "status": "success",
  "data": [
    {
      "url": "https://example.com/abc123...",
      "ox": "abc123...",
      "size": 204800,
      "mime_type": "image/jpeg",
      "distance": 3
    }
  ]
}
```

Each result is a NIP-94 event with an additional `distance` field indicating how many bits differ from the queried image's hash. Results are sorted by distance (most similar first).

Returns an error if the queried file does not yet have a perceptual hash computed.

---

## Reports

### `GET /admin/reports`

List unreviewed user-submitted content reports.

**Query parameters**

| Parameter | Type | Default | Description                 |
| --------- | ---- | ------- | --------------------------- |
| `page`    | int  | `0`     | Page number (zero-based)    |
| `count`   | int  | `50`    | Results per page (max 5000) |

**Response**

```json
{
  "status": "success",
  "data": {
    "count": 2,
    "page": 0,
    "total": 2,
    "files": [
      {
        "id": 1,
        "file_id": "abc123...",
        "reporter_id": 99,
        "event_json": "{ ... }",
        "created": "2026-01-01T00:00:00Z",
        "reviewed": false
      }
    ]
  }
}
```

---

### `DELETE /admin/reports`

Acknowledge one or more reports, marking them as reviewed without taking action on the files.

**Request body**

```json
{ "ids": [1, 2, 3] }
```

**Response**

```json
{ "status": "success" }
```

---

## Users

### `GET /admin/user/{pubkey}`

Return detailed info for a user identified by their hex-encoded pubkey.

**Query parameters**

| Parameter | Type | Default | Description                       |
| --------- | ---- | ------- | --------------------------------- |
| `page`    | int  | `0`     | Page of file results (zero-based) |
| `count`   | int  | `50`    | Files per page (max 100)          |

**Response**

```json
{
  "status": "success",
  "data": {
    "pubkey": "abc123...",
    "is_admin": false,
    "file_count": 5,
    "total_size": 2097152,
    "created": "2025-06-01T12:00:00Z",
    "files": {
      "count": 5,
      "page": 0,
      "total": 5,
      "files": [
        /* same shape as GET /admin/files, including stats */
      ]
    }
  }
}
```

---

### `DELETE /admin/user/{pubkey}/purge`

Delete all files belonging to a user. Each file's ownership records, database entry, and physical file are removed.

**Response**

```json
{ "status": "success" }
```

---

## Dynamic Config

These endpoints manage the database-backed configuration layer. Keys set here
override the same keys in `config.yaml`. Changes are picked up by the running
server within 30 seconds (DB poll interval) or immediately if the config file
also changes.

Keys use dot-notation for nested YAML paths (e.g. `max_upload_bytes`,
`payments.cost`).

### `GET /admin/config`

List all database config overrides.

**Response**

```json
{
  "status": "success",
  "data": [
    { "key": "max_upload_bytes", "value": "209715200" },
    { "key": "webhook_url",      "value": "https://hooks.example.com/notify" }
  ]
}
```

---

### `PUT /admin/config/{key}`

Set (upsert) a config override. The value is always a string; the server
parses it as boolean / integer / float / string in that order.

**Request body**

```json
{ "value": "209715200" }
```

**Response**

```json
{ "status": "success" }
```

---

### `DELETE /admin/config/{key}`

Remove a config override, reverting to the static `config.yaml` value on the
next reload.

**Response**

```json
{ "status": "success" }
```

---

## Error responses

All endpoints return a consistent error envelope on failure:

```json
{
  "status": "error",
  "message": "User is not an admin"
}
```

HTTP status `500` is used for server-side errors; auth failures return the error envelope with status `200` (check the `status` field).
