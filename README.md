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
- **AI Content Labeling** - Automated tagging using [ViT-224](https://huggingface.co/google/vit-base-patch16-224) model
- **Media Metadata** - Automatic extraction of dimensions, duration, bitrate
- **Range Request Support** - RFC 7233 compliant partial content delivery

### Security & Administration
- **Nostr Authentication** - Cryptographic identity with kind 24242 events
- **Whitelist Support** - Restrict uploads to approved public keys
- **Quota Management** - Per-user storage limits with payment integration
- **Content Reporting** - Community-driven moderation via NIP-56 reports
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
- **Comprehensive Logging** - Detailed operation tracking
- **Health Monitoring** - Service status and performance metrics

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
- `GET /admin/*` - Web dashboard for content management
- Admin API endpoints for reports and user management

## Configuration

Route96 uses YAML configuration. See [config.yaml](config.yaml) for a complete example:

```yaml
listen: "127.0.0.1:8000"
database: "mysql://user:pass@localhost:3306/route96"
storage_dir: "./data"
max_upload_bytes: 104857600  # 100MB
public_url: "https://your-domain.com"

# Optional: Restrict to specific pubkeys
whitelist: ["pubkey1", "pubkey2"]

# Optional: Payment system
payments:
  free_quota_bytes: 104857600
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
# Create authorization event (kind 24242)
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

### List user's files
```bash
curl http://localhost:8000/list/user_pubkey_hex
```

## Feature Flags

Route96 supports optional features that can be enabled at compile time:

- `nip96` (default) - NIP-96 protocol support
- `blossom` (default) - Blossom protocol support  
- `media-compression` - WebP conversion and thumbnails
- `labels` - AI-powered content labeling
- `payments` (default) - Lightning payment integration
- `analytics` (default) - Plausible analytics
- `react-ui` (default) - Web dashboard interface

```bash
# Build with specific features
cargo build --features "blossom,payments,media-compression"
```

## Requirements

- **Rust** 1.70+ 
- **MySQL/MariaDB** - Database storage
- **FFmpeg libraries** - Media processing (optional)
- **Node.js** - UI building (optional)

See [docs/debian.md](docs/debian.md) for detailed installation instructions.

## Running

### Docker Compose

The easiest way to run `route96` is to use `docker compose`

```bash
docker compose -f docker-compose.prod.yml up
```

### Docker

Assuming you already created your `config.yaml` and configured the `database` run:

```bash
docker run --rm -it \
  -p 8000:8000 \
  -v ./config.yaml:/app/config.yaml \
  -e "RUST_LOG=info" \
  voidic/route96
```

### Manual
See [install.md](docs/debian.md)