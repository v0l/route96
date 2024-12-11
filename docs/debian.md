# Debian/Ubuntu Install

**Tested on Ubuntu 24.04.1 LTS**

## Building

Install dependencies:

```bash
sudo apt install \
    libavutil-dev \
    libavformat-dev \
    libavfilter-dev \
    libavdevice-dev \
    libavcodec-dev \
    libswscale-dev \
    mariadb-server \
    clang
```

If you don't already have rust compiler installed, use rustup:

```bash
sudo apt install rustup
rustup default stable
rustup update
```

Clone the repo and build:

```bash
sudo useradd route96
sudo mkdir -p /usr/share/route96

git clone https://git.v0l.io/Kieran/route96.git
cd route96
cargo build -r
sudo cp target/release/route96 /usr/sbin/route96
```

## Build UI

The UI is a react app using yarn:

Install Node.js if you dont already have it:

```bash
curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.1/install.sh | bash
source ~/.nvm/nvm.sh
nvm install 22
```

Build the UI and copy the `index.html`

```bash
cd ui_src
npx yarn
npx yarn build
sudo mkdir -p /usr/share/route96/ui/
sudo cp dist/index.html /usr/share/route96/ui/index.html
```

## Database setup

Open mariadb-cli to your local db:

```bash
sudo mariadb
```

Run commands to create user and database:

```mysql
create user 'route96'@'localhost' identified by 'route96';
create database route96;
grant all privileges on route96.* to 'route96'@'localhost';
flush privileges;
```

## Configuration

Edit your config file to look something like this:

```yaml
listen: "0.0.0.0:8000"
database: "mysql://route96:route96@localhost:3306/route96"
storage_dir: "/usr/share/route96/data"
max_upload_bytes: 104857600
public_url: "http://localhost:8000"
```

## Systemd service

Copy the config from the cloned repo into a config directory:

```bash
sudo cp route96/config.prod.yaml /usr/share/route96/config.yaml
sudo chown -R route96:route96 /usr/share/route96
```

Systemctl config file: `/etc/systemd/system/route96.service`

```
[Unit]
Description=route96

[Service]
Type=simple
User=route96
Group=route96
WorkingDirectory=/usr/share/route96
Environment="RUST_LOG=info;rocket=error"
ExecStart=/usr/sbin/route96

[Install]
WantedBy=network.target
```

Start the service

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now route96
```

In this setup route96 will be listening on `0.0.0.0:8000`, you can modify the `listen`
config to listen on `port 80` if you don't already have a webserver running, otherwise you can add the
following `nginx` config to proxy requests.

## Nginx Reverse Proxy

```bash
sudo apt install nginx
```

Add site config: `/etc/nginx/sites-enabled/route96.conf`

```
server {
    server_name image.example.com;

    location / {
        proxy_pass http://127.0.0.1:8000;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    client_max_body_size 5g;
    proxy_read_timeout 600;

    listen 80;
    listen [::]:80;
}
```