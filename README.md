# route96

Image hosting service

## Features
- [NIP-96 Support](https://github.com/nostr-protocol/nips/blob/master/96.md)
- [Blossom Support](https://github.com/hzrd149/blossom/blob/master/buds/01.md)
  - [BUD-01](https://github.com/hzrd149/blossom/blob/master/buds/01.md)
  - [BUD-02](https://github.com/hzrd149/blossom/blob/master/buds/02.md)
  - [BUD-06](https://github.com/hzrd149/blossom/blob/master/buds/06.md)
- Image compression to WebP (FFMPEG, NIP-96 only)
- Blurhash calculation (NIP-96 only)
- AI image labeling ([ViT224](https://huggingface.co/google/vit-base-patch16-224))

## Planned
- Torrent seed V2

## Running

### Docker Compose
The easiest way to run `route96` is to use `docker compose`

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
  voidic/route96
```

## Building

### Feature Flags
Default = `nip96` & `blossom`
- `nip96`: Enable NIP-96 support
- `blossom`: Enable blossom support
- `labels`: Enable AI image labeling (Depends on `nip96`)

### Default build: 
`cargo build --release`

### Build only Blossom support
`cargo build --release --no-default-features --features blossom`

### Build dependencies
If you want to support NIP-96 you will need the following dependencies:
```bash
libavcodec-dev libavformat-dev libswscale-dev libavutil-dev libavdevice-dev libavfilter-dev
```