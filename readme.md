# transmission-rss

Subscribes to RSS feeds and adds matching torrents to Transmission. Renames downloaded files and removes old torrents automatically.

It runs as two long-running containers from one image:

- `trss-web` serves the screens and the API (channels, rules, collection history, retrying a failed item).
- `trss-worker` reads the feeds every 5 minutes, adds matching torrents, and carries out the commands the web accepts.

Both keep their state in one SQLite database (channels, rules, collection history, the library), with the work covers in a folder next to it.

## Docker Compose

### Setup

Create a `.env` file next to the compose files:

```sh
# Transmission (required)
TRANSMISSION_CONFIG_DIR=/path/to/transmission/config
MEDIA_DIR=/path/to/media
WATCH_DIR=/path/to/watch

# trss (required)
TRSS_VERSION=0.4.0            # release tag of ghcr.io/syrflover/transmission-rss
TRSS_WEB_HOST_IP=192.168.0.10 # this host's LAN address; the web listens only there

# trss (optional, defaults shown)
TRSS_DATA_DIR=./data          # app database; must be on a local disk of this host
TRSS_WEB_HOST_PORT=8080
TRSS_WORKER_INTERVAL_SECS=300
TRANSMISSION_URL=http://transmission:9091/transmission/rpc
SPEED_LIMIT_UP=0
SPEED_LIMIT_DOWN=30000
DOWNLOAD_QUEUE_SIZE=5
SEED_QUEUE_SIZE=1
```

- `MEDIA_DIR` is mounted to `/downloads` in Transmission and in both trss containers, so all of them spell folders the same way: read-write in `trss-worker`, which moves work folders when rules are archived and restored (see below), and read-only in `trss-web`, which only reads it. Transmission downloads to `/downloads/downloads` (`$MEDIA_DIR/downloads` on the host) unless a rule says otherwise.
- `TRSS_DATA_DIR` holds `trss.db`, the worker's lock files `trss.db.worker.lock`, `trss.db.artwork.lock` and `trss.db.seasons.lock`, and `artwork/`, the work covers (see [Work covers](#work-covers)). Keep it on a local disk, not an SMB or NFS share: SQLite and the locks rely on local file locking.
- The web has no sign-in of its own. `TRSS_WEB_HOST_IP` binds its port to the LAN address only; reach it from outside through a VPN, never by forwarding the port.

On a host whose Docker uses the systemd cgroup driver, install the slice the Transmission container runs in once (see [Resource limits](#resource-limits)):

```sh
sudo cp deploy/transmission.slice /etc/systemd/system/
sudo systemctl daemon-reload
```

### Run, stop, update

```sh
# Start Transmission, then trss
docker compose up -d
docker compose -f docker-compose.trss.yml up -d

# Logs
docker compose -f docker-compose.trss.yml logs -f trss-worker
docker compose -f docker-compose.trss.yml logs -f trss-web

# Stop (the database stays in TRSS_DATA_DIR)
docker compose -f docker-compose.trss.yml down

# Update: set TRSS_VERSION in .env to the new release, then
docker compose -f docker-compose.trss.yml pull
docker compose -f docker-compose.trss.yml up -d
```

Restarting one container leaves the other running: `docker compose -f docker-compose.trss.yml restart trss-web` does not pause collection.

To back up, copy `TRSS_DATA_DIR/trss.db` and `TRSS_DATA_DIR/artwork/` together while the containers are stopped (or use `sqlite3 trss.db ".backup copy.db"` while they run, then copy `artwork/`). The database records which cover file each work uses and checks the file's size and SHA-256 before showing it (again whenever the file changed), so a database restored without its `artwork/` folder shows no covers until the files are back. The containers run as root, so the files there belong to root.

### Resource limits

Each trss container is limited to 0.25 CPU and 128M of memory. The cron run of the old binary had 0.1 CPU and 96M for a job that lived a few seconds. The worker now stays up, parses every feed each cycle, and adds the selected items concurrently; the web serves the screens and previews rules against stored history. These limits are a starting point, to be revisited with `docker stats` after the first days of running.

Transmission is limited to 0.5 CPU and 512M, and runs in `transmission.slice`, which caps it softly at 384M (`MemoryHigh`). Most of that memory is page cache from writing downloads. Above the soft cap the kernel reclaims the cache and slows the writer down, and never kills it. At the hard limit, RHEL 9 kernels from 5.14.0-687.41.1 kill `transmission-daemon` while it writes, even when reclaim frees pages (RHEL-211058, reverted in RHEL-255363). The container keeps running because s6 restarts the daemon inside it, but every kill loses the progress, renames and labels saved since Transmission last wrote its resume files. Check for kills with `grep oom_kill /sys/fs/cgroup/transmission.slice/docker-*.scope/memory.events`.

### Work covers

Each work in the library has one cover: an AniList entry's cover image or a file uploaded in the work's page (tap the cover). Both containers reach AniList over HTTPS, so they need outbound access to `graphql.anilist.co` (searches) and `s4.anilist.co` (cover images); no account or key is involved.

- When the app records a work for the first time (a new folder in a watch folder, or every work of a watch folder that is added), `trss-worker` searches AniList for the work's folder name and selects an entry only when exactly one entry has that title (ignoring case and spacing) and the search was read to its end. Anything less clear stays empty for the user to choose. A rescan, a restart or opening a page never searches again. Works recorded before this version are not searched for; open the cover and use `자동으로 다시 찾기`.
- The worker sends at most one AniList request every 2 seconds, together with the web's searches, and waits as long as AniList asks when it answers `429`. Adding a watch folder with 500 works therefore takes about 20 minutes of searching in the background; collection and the web go on meanwhile. The queue lives in the database, so a restart continues it.
- Images are judged by their bytes (JPEG, PNG or WebP), up to 10 MiB and 12 million pixels (8192 pixels a side), and refused without decoding when decoding them would take more than 64 MiB (a large 16-bit PNG, a progressive JPEG, or a JPEG saved with one scan a component); they are stored under new names in `TRSS_DATA_DIR/artwork/`. Images are only fetched from AniList's image host, from addresses AniList's own answers give.
- `TRSS_ANILIST_URL` and `TRSS_ANILIST_IMAGE_ORIGINS` (comma-separated origins) override AniList's addresses, for local testing only.

### Season info

A season's air dates, episode count, studios, genres, original title and synopsis come from the AniList entries linked to the local season (one or more, in order). Both containers use the same outbound access as the covers (`graphql.anilist.co`); nothing new is needed.

- When the app records a work (or a first season) for the first time, `trss-worker` searches AniList for the folder name and links the entry only on the same exact, unique title match and complete search as for covers (up to four search requests, then one request for the entry). The link is marked `자동` and the user can change it. Works and seasons recorded before this version are not searched for; open the season and use `연결 바꾸기`. Later seasons are never linked by the app: the previous season's last entry's sequels are offered and the user confirms one.
- Entries that are not yet released or are releasing are received again once a day by the worker (one request each, at the same pace as the covers, `429` waits included). Finished entries are received again only from `정보 다시 받기`.
- The queue takes its own lock, `trss.db.seasons.lock`, so it never holds up collection or the cover queue. Descriptions are shown as plain text only.

### Watch folders and inotify

`trss-worker` watches the watch folders (including the collect and archive folders) with inotify. A new episode shows up in the library a few seconds after its file appears, without waiting for the 5-minute cycle, and only the work that changed is read again. Changes made by other containers on the same host (Transmission) are seen the same way. The cycle still reads a folder's directories by their modification times when the worker starts, when the kernel reports that it dropped events, for directories that could not be watched, and once an hour as a safety net (for example for a share that other machines change over SMB or NFS, which the kernel does not report).

Each watched directory (a watch folder, each work folder, each `Season NN` folder) takes one watch of the kernel's `fs.inotify.max_user_watches`, about 1 KB of kernel memory, counted per user for every process of that user on the host. A library of about 1,500 folders uses about 1,500 watches; the worker also opens one inotify instance per watch folder (`fs.inotify.max_user_instances`, 128 by default). When the limit is reached the watch folder's row in the settings says how many directories are not watched and why; the worker then checks those works itself every cycle, and nothing else changes. To watch them as well, raise the limit on the host, for example `echo 'fs.inotify.max_user_watches=524288' | sudo tee /etc/sysctl.d/60-inotify.conf` and `sudo sysctl --system`. Read the current value with `cat /proc/sys/fs/inotify/max_user_watches`, and see how many watches the worker holds with the `directories watched` line it logs when it starts.

### Channels and the collect folder

Channels and rules live in the app database and are edited in the web. To bring over a channel configuration of the old binary (the YAML at `CHANNELS_CONFIG_URL`), use Settings → Data → Import in the web. Importing only writes channels and rules (and the collect folder, below); it adds, renames and removes nothing.

A channel has no folder of its own. Every torrent is saved under the app's **collect folder** (Settings → Collection → Collect folder) plus its rule's save folder, so a rule with the save folder `Show/Season 01` saves to `<collect folder>/Show/Season 01`. The settings screen also takes an optional **archive folder**, where the work folders of archived rules go (below).

- Write both folders as paths the web container sees (they start with `/downloads`, the mount above). The web checks that they exist, that neither is the other or inside the other, and that they are on the same filesystem. It cannot check that Transmission can write to them; that shows up when it tries.
- Until a collect folder is set the worker adds nothing and records no failure for the items a rule picked. The status board says so, and the next cycle after the folder is set receives those items. A new database starts without one; importing a YAML file sets it (the channels' shared folder, or their common parent), and so does an upgrade from a version with per-channel folders.
- Upgrading from 0.4.x (channels with their own base folder): the migration makes the shared base folder the collect folder, or, when the channels' base folders differ, their common parent, and puts the remainder in front of each rule's save folder. Every rule keeps saving to exactly the folder it had. The old `base_dir` column is dropped, so a database opened by this version cannot go back to an older one; keep a copy of `trss.db` before upgrading. Relative base folders with nothing in common cannot be folded into one collect folder; the upgrade then refuses to start and leaves the database unchanged.
- Importing a YAML file whose channel folder is outside the collect folder reports that channel with the reason in the review step and does not import it.

### Archiving and restoring a rule

A rule's **work folder** is the first part of its save folder: `Clevatess` for `Clevatess/Season 02`. Archiving a rule (the rule's `보관` button) turns it off and then moves its work folder, `.trss/` included, from the collect folder into the archive folder; restoring it (`복원`) moves the folder back and only then turns the rule on. The web only accepts the request; `trss-worker` carries it out between cycles, under the same lock, and the rule shows how the move went.

- Transmission's torrents inside the work folder (judged by their folder's text and by where it really is, links followed) are moved first, with `torrent-set-location` (files moved), and the worker waits until Transmission reports the new folder. Then the worker renames what is left. It only renames within one filesystem, with `RENAME_NOREPLACE` (a filesystem without it is refused), so moved files keep their owner; the folders Transmission makes for its torrents belong to Transmission's user.
- Nothing moves while one of those torrents is not finished (downloading, verifying, or a magnet still fetching its metadata; finish it or remove it in Transmission, then move again), reports a local error, has a download folder written with `..` or reached through a link into or out of the work folder, or has a file the destination already holds. Files found only at the destination count as the torrent's own (moved there by hand) only when none is left at the source and each has the torrent's size. A move error Transmission reports ends the move with its text. When Transmission takes longer than the wait (5 minutes), the request stays open and the worker checks again a few seconds later, up to five times.
- When the archive folder already has the work folder (an earlier season archived before), the two are merged, season folders file by file. If any file is at the same place on both sides, nothing moves and the rule lists those files; clear one side and use `다시 옮기기`.
- The folder stays where it is when no archive folder is set, when the rule saves into the collect folder itself, or while another active rule (in any channel) still saves into the same work folder. It moves when the last of those rules is archived. A work folder that is a link, or that holds a link leading out of the two folders, is not moved.
- Where a work folder is, is read from the disk each time. A move cut short (the worker stopped or restarted) is picked up again by the worker, which moves what is left. A folder moved by hand is recognised too.
- While a rule's folder is moving, the rule's save folder cannot be changed, and no rule can be created in or moved into that work folder. A save folder with `..` is refused when it is typed (rules saved before keep working).

## Switching from the cron job

Up to 0.3.x, `scripts/cron.sh` ran the old binary every 5 minutes. Do not run it next to the worker: each removes the trss-labelled torrents that are not in its own feeds, so they can remove each other's torrents.

1. Save Transmission's torrent list to compare against later:

   ```sh
   curl -s -H "X-Transmission-Session-Id: $(curl -s -o /dev/null -w '%header{x-transmission-session-id}' http://localhost:9091/transmission/rpc)" \
     -d '{"method":"torrent-get","arguments":{"fields":["hashString","name","downloadDir","labels"]}}' \
     http://localhost:9091/transmission/rpc > torrents-before-switch.json
   ```

2. Remove the cron job and let a run in progress finish: `./scripts/cron.sh uninstall`, then check that `docker ps` shows no `trss` container.
3. Update this checkout, add `TRSS_VERSION` and `TRSS_WEB_HOST_IP` to `.env`, and start only the web: `docker compose -f docker-compose.trss.yml up -d trss-web`.
4. Import the channel configuration (Settings → Data → Import).
5. Start the worker: `docker compose -f docker-compose.trss.yml up -d trss-worker`. Its first cycle should meet every torrent the cron job added as a `duplicate` in the same folder, add only items published since the last cron run, and remove nothing it would not have removed.

### Rolling back

```sh
docker compose -f docker-compose.trss.yml stop trss-worker trss-web
./scripts/cron.sh install
```

The cron job needs `CHANNELS_CONFIG_URL` in `.env`. The app database stays as it is for another attempt.
