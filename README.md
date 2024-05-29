# void-cat-rs

Image hosting service

## Features
- [NIP-96 Support](https://github.com/nostr-protocol/nips/blob/master/96.md)
- [Blossom Support](https://github.com/hzrd149/blossom/blob/master/buds/bud-01.md)
- Image compression to WebP (FFMPEG, NIP-96)
- Blurhash calculation (NIP-96 only)
- AI image labeling ([ViT224](https://huggingface.co/google/vit-base-patch16-224))

## Planned
- Torrent seed V2

## Running

### Docker Compose
The easiest way to run `void-cat-rs` is to use `docker compose`

```bash
docker compose -f docker-compose.prod.yml up
```
### Manual
Assuming you already created your `config.toml` and configured the `database` run:
```bash
docker run --rm -it \
  -p 8000:8000 \
  -v ./config.toml:/app/config.toml \
  -e "RUST_LOG=info" \
  voidic/void-cat-rs
```