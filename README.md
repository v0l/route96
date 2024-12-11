# route96

Image hosting service

## Features

- [NIP-96 Support](https://github.com/nostr-protocol/nips/blob/master/96.md)
- [Blossom Support](https://github.com/hzrd149/blossom/blob/master/buds/01.md)
  - [BUD-01](https://github.com/hzrd149/blossom/blob/master/buds/01.md)
  - [BUD-02](https://github.com/hzrd149/blossom/blob/master/buds/02.md)
  - [BUD-05](https://github.com/hzrd149/blossom/blob/master/buds/05.md)
  - [BUD-06](https://github.com/hzrd149/blossom/blob/master/buds/06.md)
  - [BUD-08](https://github.com/hzrd149/blossom/blob/master/buds/08.md)
- Image compression to WebP
- Blurhash calculation
- AI image labeling ([ViT224](https://huggingface.co/google/vit-base-patch16-224))
- Plausible analytics

## Planned

- Torrent seed V2
- Payment system

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