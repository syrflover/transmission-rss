# transmission-rss

Subscribes to RSS feeds and adds matching torrents to Transmission. Renames downloaded files and removes old torrents automatically.

## Docker Compose

### Setup

Create a `.env` file:

```sh
# Transmission (required)
TRANSMISSION_CONFIG_DIR=/path/to/transmission/config
DOWNLOADS_DIR=/path/to/downloads
WATCH_DIR=/path/to/watch

# trss (required)
CHANNELS_CONFIG_URL=https://raw.githubusercontent.com/syrflover/syrflover/master/transmission-rss-channels.yaml

# trss (optional, defaults shown)
TRANSMISSION_URL=http://transmission:9091/transmission/rpc
SPEED_LIMIT_UP=0
SPEED_LIMIT_DOWN=30000
DOWNLOAD_QUEUE_SIZE=5
SEED_QUEUE_SIZE=1
```

### Run

```sh
# Start Transmission
docker compose up -d

# Install cron job (runs trss every 5 minutes)
./scripts/cron.sh install

# Uninstall cron job
./scripts/cron.sh uninstall
```

### Channel Configuration

[Example](https://github.com/syrflover/syrflover/blob/master/transmission-rss-channels.yaml)
