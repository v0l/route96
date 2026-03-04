# Route96

Decentralized blob storage server with Nostr integration, supporting multiple protocols and advanced media processing capabilities.

## Core Features

### Protocol Support
- **[NIP-96](https://github.com/nostr-protocol/nips/blob/master/96.md)** - Nostr file storage with media processing
- **[Blossom Protocol](https://github.com/hzrd149/blossom)** - Complete BUD specification compliance:
  - [BUD-01](https://github.com/hzrd149/blossom/blob/master/buds/01.md) - Blob retrieval (GET/HEAD)
  - [BUD-02](https://github.com/hzrd149/blossom/blob/master/buds/02.md) - Upload, delete, list operations
  - [BUD-04](https://github.com/hzrd149/blossom/blob/master/buds/04.md) - Blob mirroring from remote servers
  - [BUD-05](https://github.com/hzrd149/blossom/blob/master/buds/05.md) - Media optimization endpoints
  - [BUD-06](https://github.com/hzrd149/blossom/blob/master/buds/06.md) - Upload requirement validation
  - [BUD-08](https://github.com/hzrd149/blossom/blob/master/buds/08.md) - NIP-94 metadata support
  - [BUD-09](https://github.com/hzrd149/blossom/blob/master/buds/09.md) - Content reporting system

### Media Processing
- **Image & Video Compression** - Automatic WebP conversion and optimization
- **Thumbnail Generation** - Auto-generated thumbnails for images and videos
- **Blurhash Calculation** - Progressive image loading with blur previews
- **AI Content Labeling** - Automated tagging using configurable HuggingFace ViT models
- **Media Metadata** - Automatic extraction of dimensions, duration, bitrate
- **Range Request Support** - RFC 7233 compliant partial content delivery

### Content Moderation
- **Automated Flagging** - Files are automatically flagged (`LabelFlagged`) when AI labels match configured terms
- **User Reports** - Community-driven reporting via NIP-56; flagged files get `Reported` state
- **Review Queue** - Admin API to list all files pending review (`LabelFlagged` or `Reported`)
- **Admin Actions** - Mark files as reviewed or hard-delete them directly from the queue
- **Background Labeling** - Retroactively labels existing uploads that were missed at upload time

### Security & Administration
- **Nostr Authentication** - Cryptographic identity with kind 24242 events
- **Whitelist Support** - Restrict uploads to approved public keys
- **Quota Management** - Per-user storage limits with payment integration
- **Admin Dashboard** - Web interface for content and user management
- **CORS Support** - Full cross-origin resource sharing compliance
- **EXIF Privacy Protection** - Optional rejection of images with sensitive metadata (GPS, device info)

### Payment System
- **Lightning Network** - Bitcoin payments via LND integration
- **Fiat Tracking** - Multi-currency support (USD/EUR/GBP/JPY/etc.)
- **Flexible Billing** - Usage-based pricing (storage, egress, time-based)
- **Free Quotas** - Configurable free tier for new users

### Analytics & Monitoring
- **Plausible Integration** - Privacy-focused usage analytics
- **Comprehensive Logging** - Detailed operation tracking with per-label confidence scores

## API Endpoints

### Blossom Protocol
- `GET /<sha256>` - Retrieve blob by hash
- `HEAD /<sha256>` - Check blob existence
- `PUT /upload` - Upload new blob
- `DELETE /<sha256>` - Delete owned blob
- `GET /list/<pubkey>` - List user's blobs
- `PUT /mirror` - Mirror blob from remote URL
- `PUT /media` - Upload with media optimization
- `HEAD /upload` - Validate upload requirements
- `PUT /report` - Submit content reports

### NIP-96 Protocol
- `GET /.well-known/nostr/nip96.json` - Server information
- `POST /nip96` - File upload with Nostr auth
- `DELETE /nip96/<sha256>` - Delete with Nostr auth

### Admin Interface
- `GET /admin/self` - Current user info and quota
- `GET /admin/files` - List all files (paginated, filterable by MIME type)
- `GET /admin/files/review` - List files pending moderation review
- `PATCH /admin/files/<sha256>/review` - Mark a file as reviewed (clears flag)
- `DELETE /admin/files/<sha256>/review` - Delete a flagged file from disk and DB
- `GET /admin/reports` - List unreviewed user reports
- `DELETE /admin/reports/<id>` - Acknowledge a report
- `GET /admin/user/<pubkey>` - User info, files, and payment history
- `DELETE /admin/user/<pubkey>/purge` - Delete all files belonging to a user
- `GET /docs.md` - Admin API reference (embedded Markdown)

## Configuration

Route96 uses YAML configuration. See [config.yaml](config.yaml) for a complete example.

### Minimal config

```yaml
listen: "127.0.0.1:8000"
database: "mysql://user:pass@localhost:3306/route96"
storage_dir: "./data"
max_upload_bytes: 104857600  # 100MB
public_url: "https://your-domain.com"
```

### AI Labeling & Auto-moderation

```yaml
# Directory where HuggingFace model files are cached (default: <storage_dir>/models)
models_dir: "./data/models"

# Models to run on every upload. Files are downloaded from HuggingFace on first use.
label_models:
  - hf_repo: "google/vit-base-patch16-224"
    name: "vit224"
  - hf_repo: "Falconsai/nsfw_image_detection"
    name: "nsfw"
    label_exclude:
      - "normal"   # suppress the "clean" class from the NSFW model

# Labels containing any of these terms (case-insensitive substring) will be
# automatically flagged for admin review.
label_flag_terms:
  - "nsfw"
  - "explicit"
  - "porn"
  - "sexy"
  - "hentai"
```

Models are downloaded once and cached under `models_dir/<org>--<repo>/`. A background
task runs at startup to retroactively label any existing uploads that are missing labels.

### Payment system

```yaml
payments:
  free_quota_bytes: 104857600  # 100 MB free tier
  fiat: "USD"
  lnd:
    endpoint: "https://127.0.0.1:10001"
    tls: "/path/to/tls.cert"
    macaroon: "/path/to/admin.macaroon"
  cost:
    currency: "BTC"
    amount: 0.00000100
  unit: "GBSpace"
  interval:
    month: 1
```

## Quick Start Examples

### Upload a file (Blossom)
```bash
auth_event='{"kind":24242,"tags":[["t","upload"],["expiration","1234567890"]],"content":"Upload file"}'
auth_b64=$(echo $auth_event | base64 -w 0)

curl -X PUT http://localhost:8000/upload \
  -H "Authorization: Nostr $auth_b64" \
  -H "Content-Type: image/jpeg" \
  --data-binary @image.jpg
```

### Retrieve a file
```bash
curl http://localhost:8000/abc123def456...789
```

### Review the moderation queue
```bash
# List files pending review
curl -H "Authorization: Nostr $auth_b64" \
  http://localhost:8000/admin/files/review

# Mark a file as reviewed
curl -X PATCH -H "Authorization: Nostr $auth_b64" \
  http://localhost:8000/admin/files/<sha256>/review

# Delete a flagged file
curl -X DELETE -H "Authorization: Nostr $auth_b64" \
  http://localhost:8000/admin/files/<sha256>/review
```

## Feature Flags

| Flag | Default | Description |
|------|---------|-------------|
| `nip96` | on | NIP-96 protocol support |
| `blossom` | on | Blossom protocol support |
| `media-compression` | off | WebP conversion, thumbnails, media metadata |
| `labels` | on | AI content labeling (requires `media-compression`) |
| `cuda` | off | Run label models on GPU via CUDA (implies `labels`) |
| `payments` | off | Lightning payment integration |
| `analytics` | on | Plausible analytics |
| `react-ui` | on | Web dashboard interface |
| `r96util` | on | CLI utility binary for maintenance operations |

```bash
# CPU labeling
cargo build --features "blossom,media-compression,labels"
# GPU labeling
cargo build --features "blossom,media-compression,cuda"
```

## Requirements

- **Rust** 1.70+
- **MySQL/MariaDB** - Database storage
- **FFmpeg libraries** - Media processing (`media-compression` / `labels` features)
- **Node.js** - UI building (optional)

See [docs/debian.md](docs/debian.md) for detailed installation instructions.

## Running

### Docker Compose

```bash
docker compose -f docker-compose.prod.yml up
```

### Docker

```bash
docker run --rm -it \
  -p 8000:8000 \
  -v ./config.yaml:/app/config.yaml \
  -e "RUST_LOG=info" \
  voidic/route96
```

### Manual
See [docs/debian.md](docs/debian.md)
