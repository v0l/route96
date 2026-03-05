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

## Files

### `GET /admin/files`
List all files (excluding banned), newest first.

**Query parameters**

| Parameter   | Type   | Default | Description                                        |
|-------------|--------|---------|----------------------------------------------------|
| `page`      | int    | `0`     | Page number (zero-based)                           |
| `count`     | int    | `50`    | Results per page (max 5000)                        |
| `mime_type` | string | —       | Filter by MIME type substring                      |
| `label`     | string | —       | Filter to files whose labels contain this substring|

**Response**
```json
{
  "status": "success",
  "data": {
    "count": 10,
    "page": 0,
    "total": 42,
    "files": [ /* NIP-94 events with uploader pubkeys */ ]
  }
}
```

---

## Moderation Queue

### `GET /admin/files/review`
List files pending moderation review — those with `review_state` of `LabelFlagged` (1) or `Reported` (2), oldest first.

**Query parameters**

| Parameter | Type | Default | Description                   |
|-----------|------|---------|-------------------------------|
| `page`    | int  | `0`     | Page number (zero-based)      |
| `count`   | int  | `50`    | Results per page (max 5000)   |

**Response** — same shape as `GET /admin/files`.

---

### `PATCH /admin/files/{sha256}/review`
Mark a file as reviewed, clearing the moderation flag (`review_state → Reviewed`). The file remains accessible.

**Response**
```json
{ "status": "success" }
```

---

### `DELETE /admin/files/{sha256}/review`
Permanently ban a file. The physical file is removed from disk, all ownership records are deleted, and the database row is kept as a tombstone so the same hash cannot be re-uploaded.

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

| Parameter  | Type | Default | Description                                        |
|------------|------|---------|----------------------------------------------------|
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

| Parameter | Type | Default | Description                   |
|-----------|------|---------|-------------------------------|
| `page`    | int  | `0`     | Page number (zero-based)      |
| `count`   | int  | `50`    | Results per page (max 5000)   |

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

### `DELETE /admin/reports/{id}`
Acknowledge a report, marking it as reviewed without taking action on the file.

**Response**
```json
{ "status": "success" }
```

---

## Users

### `GET /admin/user/{pubkey}`
Return detailed info for a user identified by their hex-encoded pubkey.

**Query parameters**

| Parameter | Type | Default | Description                          |
|-----------|------|---------|--------------------------------------|
| `page`    | int  | `0`     | Page of file results (zero-based)    |
| `count`   | int  | `50`    | Files per page (max 100)             |

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
    "files": { "count": 5, "page": 0, "total": 5, "files": [] }
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

## Error responses

All endpoints return a consistent error envelope on failure:

```json
{
  "status": "error",
  "message": "User is not an admin"
}
```

HTTP status `500` is used for server-side errors; auth failures return the error envelope with status `200` (check the `status` field).
