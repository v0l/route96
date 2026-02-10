# AGENTS.md - Route96 Development Guide

Route96 is a decentralized blob storage server with Nostr integration, supporting NIP-96 and Blossom protocols.

## Project Structure

```
route96/
├── src/                    # Rust backend
│   ├── bin/main.rs         # Application entry point
│   ├── lib.rs              # Library root with module declarations
│   ├── routes/             # HTTP route handlers (blossom, nip96, admin, payment)
│   ├── auth/               # Authentication (blossom.rs, nip98.rs)
│   ├── db.rs               # Database models and queries (SQLx + MySQL)
│   ├── filesystem.rs       # File storage operations
│   ├── processing/         # Media processing (compression, labeling)
│   └── settings.rs         # Configuration structures
├── ui_src/                 # React/TypeScript frontend
│   └── src/
│       ├── views/          # Page components
│       ├── components/     # Reusable UI components
│       └── upload/         # Upload utilities (blossom.ts, nip96.ts)
└── migrations/             # SQL migration files
```

## Build Commands

### Rust Backend
```bash
cargo build                 # Debug build
cargo build -r              # Release build
cargo build --features "blossom,payments,media-compression"
cargo run -- --config config.yaml
```

### Feature Flags
- `nip96` (default) - NIP-96 protocol (requires media-compression)
- `blossom` (default) - Blossom protocol
- `analytics` (default) - Plausible analytics
- `react-ui` (default) - Web dashboard
- `media-compression` - WebP conversion, thumbnails (requires FFmpeg)
- `labels` - AI content labeling (requires media-compression)
- `payments` - Lightning payment integration

### TypeScript Frontend (ui_src/)
```bash
yarn           # Install dependencies
yarn dev       # Development server
yarn build     # Production build (tsc -b && vite build)
```

## Testing

```bash
cargo test                              # Run all tests
cargo test test_name                    # Run single test by name
cargo test module::test_name            # Run test with module path
cargo test -- --nocapture               # Show test output
cargo test --features "blossom"         # Test specific features
```

## Linting and Formatting

### Rust
```bash
cargo fmt                   # Format code
cargo fmt --check           # Check formatting
cargo clippy                # Lint
cargo clippy --all-features
```

### TypeScript (ui_src/)
```bash
yarn prettier --check src/
yarn prettier --write src/
```

## Rust Code Style

**Naming:** `snake_case` functions/variables, `PascalCase` types, `SCREAMING_SNAKE_CASE` constants

**Imports:** External crates first, then local modules (`crate::`)
```rust
use anyhow::{Error, Result};
use axum::{Json, Router, extract::State};
use serde::{Deserialize, Serialize};

use crate::db::FileUpload;
use crate::settings::Settings;
```

**Error Handling:** Use `anyhow::Result<T>`, `?` operator, `Error::msg("description")`
```rust
pub fn example() -> Result<()> {
    let file = File::open(path)?;
    if !valid {
        return Err(Error::msg("Invalid file format"));
    }
    Ok(())
}
```

**Feature Flags:** Use `#[cfg(feature = "...")]` for conditional compilation
```rust
#[cfg(feature = "payments")]
pub mod payments;
```

**Structs:** Derive `Clone`, `Serialize`, `Deserialize`; use `#[serde(skip_serializing_if = "Option::is_none")]` for optional fields; `#[sqlx(skip)]` for non-database fields

**Async:** Use Tokio runtime; prefer `tokio::fs` over `std::fs` in async contexts

## TypeScript Code Style

**Naming:** `camelCase` variables/functions, `PascalCase` components/classes

**Imports:** External packages first, then local modules
```typescript
import { useState } from "react";
import { EventPublisher } from "@snort/system";
import { Blossom } from "../upload/blossom";
```

**Components:** Functional components with hooks, TypeScript interfaces for props

## Database

- MySQL/MariaDB with SQLx
- Migrations in `migrations/` (auto-applied via `sqlx::migrate!`)
- Connection: `mysql://user:pass@localhost:3306/route96`

## Configuration

Runtime config via `config.yaml`:
```yaml
listen: "127.0.0.1:8000"
database: "mysql://user:pass@localhost:3306/route96"
storage_dir: "./data"
max_upload_bytes: 104857600
public_url: "https://your-domain.com"
```

## Environment Variables

- `RUST_LOG` - Logging level (`info`, `debug`)
- `APP_*` - Override config values (e.g., `APP_DATABASE`)

## Key Dependencies

**Rust:** axum, tokio, sqlx (MySQL), nostr, serde, anyhow
**TypeScript:** React 19, Vite 7, Tailwind CSS 4, @snort/system
