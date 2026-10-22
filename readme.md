# transmission-rss

Subscribes to RSS feeds and adds matching torrents to Transmission. Renames downloaded files and removes old torrents automatically.

It runs as two long-running containers, each from its own image:

- `trss` serves the screens and the API (channels, rules, collection history, retrying a failed item), reads the feeds every 5 minutes, adds matching torrents, and carries out the commands the screens send, in one process.
- `trss-browser` is the server browser that subtitle jobs open when a source needs a person to pass a check (see [`trss-browser`](#trss-browser)).

trss keeps its state in one SQLite database (channels, rules, collection history, the library), with the work covers in a folder next to it.

Until release 0.7.0, the web (`trss-web`) and the worker (`trss-worker`) were two containers from one image; [Run, stop, update](#run-stop-update) says how an update replaces them.

## Docker Compose

### Requirements

Docker Engine 28 or later with Docker Compose 2.33.1 or later. The compose file gives `trss` its default gateway with `gw_priority` (see [`trss-browser`](#trss-browser)), which older versions do not have. A `trss` whose default route still goes through the server browser's network does not start, and its log names `gw_priority` and these versions.

Not tested:

- Any Docker Engine or Compose version other than the ones in the list below (as of 2026-10-07).

<details>
<summary>Tested versions</summary>

Tested:

- Compose 5.5.1 with Docker Engine 29.8.1 and 29.8.2 (2026-10-07).

</details>

### Setup

Create a `.env` file next to the compose files:

```dotenv
# Transmission (required)
TRANSMISSION_CONFIG_DIR=/path/to/transmission/config
MEDIA_DIR=/path/to/media
WATCH_DIR=/path/to/watch

# trss (required)
TRSS_VERSION=0.8.0            # release tag of ghcr.io/syrflover/transmission-rss
TRSS_WEB_HOST_IP=192.168.0.10 # this host's LAN address; the web listens only there
TRSS_BROWSER_TOKEN=<random>   # what trss and the browser container share, e.g. from `openssl rand -hex 32`

# trss (optional, defaults shown)
TRSS_DATA_DIR=./data          # app database; must be on a local disk of this host, and exist and belong to TRSS_UID:TRSS_GID before the first start
TRSS_UID=1000                 # the user the trss container runs as, see "Running as 1000:1000"
TRSS_GID=1000
TRSS_WEB_HOST_PORT=8080
TRSS_WEB_HOSTS=               # host names the web answers to besides IP addresses and localhost, comma-separated
TRSS_WORKER_INTERVAL_SECS=300
TRANSMISSION_URL=http://transmission:9091/transmission/rpc
SPEED_LIMIT_UP=0
SPEED_LIMIT_DOWN=30000
DOWNLOAD_QUEUE_SIZE=5
SEED_QUEUE_SIZE=1
```

On a host whose Docker uses the systemd cgroup driver, install the slice the Transmission container runs in once, in the folder with the compose files (see [Resource limits](#resource-limits)):

```sh
sudo cp deploy/transmission.slice /etc/systemd/system/ &&
sudo systemctl daemon-reload
```

The next four parts explain the settings and the folders behind them.

#### `MEDIA_DIR`

`MEDIA_DIR` is mounted to `/downloads` in Transmission and in `trss`, so both spell folders the same way. Transmission downloads to `/downloads/downloads` (`$MEDIA_DIR/downloads` on the host) unless a rule says otherwise.

`trss` mounts it read-write: it moves work folders when rules are archived and restored, as [Archiving and restoring a rule](#archiving-and-restoring-a-rule) describes, and places subtitles in them. Its API handlers do not change media files; only the commands, the subtitle jobs and the cleanups of stored files do.

#### `TRSS_DATA_DIR`

Keep it on a local disk, not an SMB or NFS share: SQLite and the lock rely on local file locking. Make it, and `browser-downloads` in it, before the first start and give them to the user the container runs as; the commands are in [Running as 1000:1000](#running-as-10001000). A folder Docker makes for a missing path belongs to root, which the container cannot write.

It holds:

- `trss.db`
- `trss.db.lock`, which the running `trss` holds so that a second one on the same folder stops at its start
- `receive/`, where subtitle jobs put what they receive
- `artwork/`, the work covers (see [Work covers](#work-covers))
- `subtitle-files/`, the descriptions, licences and companion files of the subtitle packages trss keeps (subtitles and fonts stay in each work folder's `.trss/`)

#### The web's access

The web of `trss` has no sign-in of its own. `TRSS_WEB_HOST_IP` binds its port to the LAN address only; reach it from outside through a VPN, never by forwarding the port.

It answers requests addressed to an IP address or `localhost`, and to the names listed in `TRSS_WEB_HOSTS` (none by default): a request for any other host name is refused (`421`), so that a page of another name made to resolve to this address (DNS rebinding) cannot use it.

A request that changes something, and every WebSocket, is refused (`403`) when its `Origin` is not the web's own or is `null`.

Reaching the web by LAN IP and port needs no setting. A reverse proxy that serves it under a name must pass the browser's `Host` on (nginx: `proxy_set_header Host $host`), and that name must be in `TRSS_WEB_HOSTS`.

#### `trss-browser`

`trss-browser` is the server browser for the subtitle sources that need a person to pass a check (`Dockerfile.browser`, image `ghcr.io/syrflover/trss-browser`, tagged with the same release as the app). It is always up, but idle it holds only a virtual display and a small launcher (`trss-browserd`); `trss` asks the launcher to start a Chromium for a job (with a profile of its own, deleted when the run ends) and to close it.

Keep these as the compose file has them:

- It publishes no port, and it should stay that way: neither the launcher nor Chromium's DevTools is reachable from the host or the LAN, and the launcher answers only requests with `TRSS_BROWSER_TOKEN`.
- Keep other services off `browser_net`, the compose network the browser shares with `trss`.
- The published port of `trss` must come in through `trss_net`: the compose file makes `trss_net` the default gateway of `trss` with `gw_priority`, which needs Docker Engine 28 or later.
- One `trss` at a time uses one `trss-browser`: `trss` resets the launcher when it starts, and a second would end the first one's runs. A second `trss` on the same app data stops at its start, before it touches the browser (the lock file `trss.db.lock`, see [`TRSS_DATA_DIR`](#trss_data_dir)).

Since a bridge network still reaches this host's LAN address, the Docker gateway and the LAN, every connection of the browser goes through the launcher's egress proxy, which connects only to public addresses. It cannot tell a public address that leads back into the LAN, such as the router's WAN address with hairpin NAT or your own domain pointing at it. A host firewall rule that drops what `browser_net` sends to private ranges is an optional extra (see the comment in `docker-compose.trss.yml`).

<details>
<summary>Why the browser has its own network, and what the egress proxy refuses</summary>

- The browser opens the pages of the sites it is sent to, so it is not on `trss_net` (where Transmission listens): it has the compose network `browser_net`, shared with `trss`, which is on both.
- `trss` starts and ends runs, and connects to a run's page through the launcher's token-protected DevTools proxy to show a job's remote screen, where a person passes a site's check.
- The web has no sign-in, so `trss` refuses every connection from `browser_net`'s addresses, and does not start when it cannot find them (it resolves `TRSS_BROWSER_URL` at its start).
- Docker routes a container's published ports through the network of its default gateway and would otherwise pick `browser_net`, whose gateway address `trss` refuses; this is why the compose file sets `gw_priority`.
- `browser_net` is not `internal`, since the pages need the internet.
- The egress proxy resolves names itself and refuses private, loopback, link-local, CGNAT (Tailscale), multicast and reserved addresses, IPv4 and IPv6, and the networks the container is on with their gateways whatever their addresses, so a name that resolves to a LAN address is refused as well.
- Chromium runs with QUIC off and without WebRTC UDP outside the proxy.

</details>

`trss` reads these variables for the browser:

| Variable | Value |
| --- | --- |
| `TRSS_BROWSER_URL` | `http://trss-browser:9230`, set in the compose file |
| `TRSS_BROWSER_TOKEN` | The value in `.env` |
| `TRSS_BROWSER_DOWNLOADS` | `/data/browser-downloads` |

Without `TRSS_BROWSER_URL` it runs without a browser and shows no remote screen. How long a browser may sit idle and how many jobs may use one at once are in the settings' common policy.

Downloads land in `TRSS_DATA_DIR/browser-downloads/`, which `trss` empties once the reset of the launcher at its start has gone through (if the container cannot be reached then, at the first use). The browser writes there as uid 10001, so `trss` opens the folder to it (mode 1777) at its start, which it can do only for a folder it owns (see [Running as 1000:1000](#running-as-10001000)).

These downloads are canceled:

- a download over 200 MiB;
- a run whose downloads pass 1 GiB;
- a download that reports no progress for 120 s.

### Run, stop, update

Run the commands of this section on the host, in the folder with the compose files.

#### Start

Start Transmission, then `trss`:

```sh
docker compose up -d &&
docker compose -f docker-compose.trss.yml up -d
```

#### Logs

Follow the log of `trss`:

```sh
docker compose -f docker-compose.trss.yml logs -f trss
```

#### Stop

Stop `trss`; the database stays in `TRSS_DATA_DIR`:

```sh
docker compose -f docker-compose.trss.yml down
```

The screens and the collection run in one process: restarting `trss` stops both for the few seconds it takes.

#### Update

1. Set `TRSS_VERSION` in `.env` to the new release.
2. Bring the compose files to it by running `git pull` in the clone of this repository that holds them:

   ```sh
   git pull
   ```

3. Pull the new images and start `trss` with them:

   ```sh
   docker compose -f docker-compose.trss.yml pull &&
   docker compose -f docker-compose.trss.yml up -d --remove-orphans
   ```

`--remove-orphans` removes the containers of services the compose file no longer has. An update from release 0.7.0 or before needs it: the file then had `trss-web` and `trss-worker`, which this one replaces with `trss`, and without it the two keep running and hold the web's port, so `trss` does not start. Afterwards this lists only `trss` and `trss-browser`:

```sh
docker ps --filter name=trss
```

If `up -d` ran without `--remove-orphans` first, the `trss` container it made is on `trss_net` alone and keeps restarting with `cannot resolve the server browser's host`, and a later `up -d --remove-orphans` keeps that container. Make it again:

```sh
docker compose -f docker-compose.trss.yml up -d --remove-orphans --force-recreate
```

#### Torrent list before and after an update

To see what an update did to Transmission, list its torrents before the update and compare the list with a second one after the first cycle. A line that went, came or changed is a torrent removed, added, moved or renamed.

Before the update, in the folder with the compose files:

```sh
python3 deploy/torrent-list.py > torrents-before.tsv
```

After the first cycle:

```sh
python3 deploy/torrent-list.py > torrents-after.tsv &&
diff torrents-before.tsv torrents-after.tsv
```

Each line holds the hash, download folder, name and labels of a torrent, sorted by hash; the script needs only Python's standard library. It reads Transmission at `http://127.0.0.1:9091/transmission/rpc`; give another RPC URL, with `user:password@` if it asks for a login, as its argument.

<details>
<summary>Tested runs</summary>

Tested:

- Both commands against a stub that answers Transmission's `torrent-get` with two torrents, the script given the stub's URL as its argument: the two lists were identical and `diff` printed nothing (2026-10-22).

</details>

#### Back up

Back up the database, `artwork/` and `subtitle-files/` of `TRSS_DATA_DIR` together. The database records which cover file each work uses and checks the file's size and SHA-256 before showing it (again whenever the file changed), so a database restored without its `artwork/` folder shows no covers until the files are back.

The files there belong to the user the container runs as (1000:1000 by default). Copy them with that user's rights, or as root and then check the owner when you put them back (see [Running as 1000:1000](#running-as-10001000)). The blocks below copy as root with `sudo`.

Run the blocks below in the same terminal, in the folder with the compose files. The first one reads the data folder the way Compose does from the compose file and `.env`, and prints it.

```sh
TRSS_CONFIG="$(docker compose -f docker-compose.trss.yml config --format json)" &&
TRSS_DATA="$(printf '%s' "${TRSS_CONFIG:?run the first line of this block first}" | python3 -c 'import json,sys; s=json.load(sys.stdin)["services"]["trss"]; print([v["source"] for v in s["volumes"] if v["target"]=="/data"][0])')" &&
echo "data folder: ${TRSS_DATA}"
```

The copies below take the existing folder to copy into from `TRSS_BACKUP`, which you write in single quotes after the `=` before you run the block.

Not tested:

- The copies with a real `sudo` on files owned by another user, and `sqlite3 .backup` while `trss` runs and writes (2026-10-22).

While the containers are stopped, copy the database and the two folders:

```sh
TRSS_BACKUP= &&
sudo cp -a "${TRSS_DATA:?run the previous block first}/trss.db" "${TRSS_DATA:?run the previous block first}/artwork" "${TRSS_DATA:?run the previous block first}/subtitle-files" "${TRSS_BACKUP:?set TRSS_BACKUP first}/"
```

While the containers run, back up the database with SQLite and copy the two folders (write `TRSS_BACKUP` the same way):

```sh
TRSS_BACKUP= &&
sudo sqlite3 "${TRSS_DATA:?run the previous block first}/trss.db" ".backup '${TRSS_BACKUP:?set TRSS_BACKUP first}/trss.db'" &&
sudo cp -a "${TRSS_DATA:?run the previous block first}/artwork" "${TRSS_DATA:?run the previous block first}/subtitle-files" "${TRSS_BACKUP:?set TRSS_BACKUP first}/"
```

<details>
<summary>Tested runs</summary>

Tested:

- The first block, then both copy blocks, on a scratch copy of the compose file with a scratch data folder holding a SQLite database, `artwork/` and `subtitle-files/`, run in zsh as uid 1000 with `sudo` replaced by a function that runs the command directly: the data folder was printed, the files and the table came across, and an unset `TRSS_BACKUP` stopped the command with its message (2026-10-22).

</details>

### Running as 1000:1000

`trss` runs as uid:gid 1000:1000, the user Transmission writes the media as (`PUID` and `PGID` in `docker-compose.yml`). The players read the media over NFS, which shows a file's owner as numbers, so what trss writes into the media (the `.trss/` folders, subtitles and fonts it places, work folders it creates) has the same owner as the videos next to it.

A host whose Transmission runs as another user sets `TRSS_UID` and `TRSS_GID` in `.env` and gives the data folder to them.

`trss` checks at its start that it can write what it writes in `TRSS_DATA_DIR`:

- the folder itself;
- the files next to the database (`trss.db`, its `-wal`, `-shm` and `-journal`, and the lock file);
- `receive/`, `artwork/` and `subtitle-files/` with every folder in them.

Nothing else in the folder is looked at:

- `browser-downloads/` (below);
- a `lost+found` that a disk of its own has;
- a copy of the database you made.

The files inside those folders are not checked either: `receive/` holds downloads of the browser container with their owner, uid 10001, which trss can read and remove. If something is another user's, it prints what and who and exits with an error instead of failing later in pieces:

```text
trss: cannot write the app data folder /data as uid 1000 gid 1000
  /data/trss.db.lock: owned by uid 0 gid 0, mode 0644 (Permission denied (os error 13))
Give these files to uid 1000 gid 1000 (for example `chown -R 1000:1000` on the folder that holds them), or run this process as their owner.
```

`TRSS_DATA_DIR/browser-downloads/` is shared with the browser container, which writes there as uid 10001, so it needs mode 1777, and it has to belong to trss: with the sticky bit only a folder's owner can remove the run folders the browser makes in it, and they would pile up. trss sets the mode at its start when it owns the folder.

When the folder belongs to another user, whatever its mode (Docker makes a missing one as root, and a root worker of an earlier release left one), trss does not stop: it prints the path, the owner and the commands below, and runs without the server browser, so the jobs that need it wait.

Make the folder with the commands below before the first `up`. Run them in the same terminal, on the host, in the folder with `docker-compose.trss.yml`. They need `sudo` unless you are uid 1000 (see `id -u`). The first block reads the data folder and the owner (`TRSS_UID:TRSS_GID`) the way Compose does from the compose file and `.env`, and prints them.

```sh
TRSS_CONFIG="$(docker compose -f docker-compose.trss.yml config --format json)" &&
TRSS_DATA="$(printf '%s' "${TRSS_CONFIG:?run the first line of this block first}" | python3 -c 'import json,sys; s=json.load(sys.stdin)["services"]["trss"]; print([v["source"] for v in s["volumes"] if v["target"]=="/data"][0])')" &&
TRSS_OWNER="$(printf '%s' "${TRSS_CONFIG:?run the first line of this block first}" | python3 -c 'import json,sys; print(json.load(sys.stdin)["services"]["trss"]["user"])')" &&
echo "data folder: ${TRSS_DATA}  owner: ${TRSS_OWNER}"
```

Check the printed pair, then make the folders with the second block.

Not tested:

- The second block with a real `sudo` (2026-10-22).

```sh
sudo mkdir -p "${TRSS_DATA:?run the previous block first}/browser-downloads" &&
sudo chown "${TRSS_OWNER:?run the previous block first}" "${TRSS_DATA:?run the previous block first}" "${TRSS_DATA:?run the previous block first}/browser-downloads" &&
sudo chmod 1777 "${TRSS_DATA:?run the previous block first}/browser-downloads"
```

If `trss` already runs without the server browser, restart it after the commands:

```sh
docker compose -f docker-compose.trss.yml restart trss
```

<details>
<summary>Tested runs</summary>

Tested:

- Both blocks in zsh and in bash, on a scratch copy of the compose file and a `.env` without `TRSS_DATA_DIR`, as uid 1000 with `sudo` replaced by a function that runs the command directly: the printed pair was the scratch `data` folder and `1000:1000`, and the folders came out owned by 1000:1000 with mode 1777 on `browser-downloads` (2026-10-22).
- The first block with `TRSS_UID=1500`, `TRSS_GID=1600` and `TRSS_DATA_DIR=./other` added to `.env`: it printed the `other` folder under the compose folder and `1500:1600` (2026-10-22).
- The second block pasted alone in a new zsh: it stopped with `run the previous block first` before creating anything (2026-10-22).

</details>

### File probe

`trss-probe` checks, on a host's real disks, the file behavior that archiving and subtitle placing rely on, and prints a plain-text report. It makes only folders and files named `.trss-probe-*` and removes them. It checks:

- the filesystem and mount options of the media and data folders;
- `renameat2(RENAME_NOREPLACE)` (and its refusal with `EEXIST`);
- `EXDEV` between the data folder and the media;
- `fsync`;
- the stability of `dev:inode` across a rename;
- the owner and mode of what it makes (expected 1000:1000);
- a test subfolder in an existing work folder;
- a 300 MiB write and read under the container's memory limit, with the cgroup's `memory.current` peak and `oom_kill`.

With `--unpack FILE[,FILE...]` (repeatable; the volumes of a split archive comma-separated, in order) it also unpacks each archive in a test folder under the data folder, as trss does with `trss-extract`, and reports the outcome, the time, the memory peaks and the new `oom_kill` events (a new kill fails the check; a refusal does not).

Releases after 0.5.0 carry it as `/usr/local/bin/trss-probe`. For a host that runs an older image, build the static binary and run it in a one-off container that looks like the one `trss` runs in (user 1000:1000, 384M of memory, no swap, its mounts). Run the blocks below in the same terminal.

Not tested:

- The probe runs in steps 3 and 4, which start the one-off container from the release image and need a work folder and sample archives on a host (2026-10-22).
- The `docker build` in step 1 (2026-10-22).

1. In the clone of this repository, build the static binary; it writes `probe-out/trss-probe` and `probe-out/trss-extract`:

   ```sh
   docker build --target probe-binary --output type=local,dest=probe-out .
   ```

2. Copy `probe-out/trss-probe`, `probe-out/trss-extract` and `deploy/probe.sh` to the compose folder on the host. The command below copies on one machine, so use another way when you built elsewhere. Write the compose folder in single quotes after the `=` of `TRSS_COMPOSE` before you run it:

   ```sh
   TRSS_COMPOSE= &&
   cp probe-out/trss-probe probe-out/trss-extract deploy/probe.sh "${TRSS_COMPOSE:?set TRSS_COMPOSE first}/"
   ```

3. Go to the compose folder and probe a work folder. Write the work folder's path in single quotes after the `=` of `TRSS_WORK` before you run it. Use the path the `trss` container sees: it starts with `/downloads`, then the collect folder, then a work folder in it:

   ```sh
   TRSS_WORK= &&
   cd "${TRSS_COMPOSE:?run step 2 first}" &&
   ./probe.sh -- --work "${TRSS_WORK:?set TRSS_WORK first}" 2>&1 | tee probe.txt
   ```

4. To try the unpacking, run it on `big.rar` from the `~/samples` folder:

   ```sh
   ./probe.sh --samples ~/samples -- --size-mib 0 --unpack /samples/big.rar 2>&1 | tee unpack.txt
   ```

The options of `probe.sh` include:

- `--samples DIR` mounts a host folder read-only at `/samples`;
- `--extract FILE` names another `trss-extract` than the one next to the script.

`./probe.sh --help` lists the options. The steps for the server are in ticket [4-store-apply-and-replace/0060](docs/archive/tickets/4-store-apply-and-replace/0060-server-filesystem-probe.md).

<details>
<summary>Tested runs</summary>

Tested:

- Step 2 with a scratch folder as `TRSS_COMPOSE`: the three files arrived executable, and `./probe.sh --help` printed its options there (2026-10-22).
- Step 3's `TRSS_WORK= &&` line with the value left empty: the `:?` message stopped the command before the probe started (2026-10-22).

</details>

### Resource limits

`trss-browser` is limited to 768M of memory, with no CPU limit. A run opens a Chromium, so give it more when the policy allows several concurrent browser jobs. Ticket [3-subtitle-candidates-and-receiving/0032](docs/archive/tickets/3-subtitle-candidates-and-receiving/0032-server-browser-spike.md) measured the memory limit with one Chromium; CPU was not measured under a limit.

`trss` is limited to 0.5 CPU and 384M of memory, the sums of the web's and the worker's limits when they were two containers (0.25 CPU each, 128M and 256M). These limits are a starting point, to be revisited with `docker stats` once the one process has run for some days.

It unpacks a received subtitle archive in a child process (`trss-extract`, beside it in the image) that shares its limit, with its own address space held to 256 MiB and its `oom_score_adj` raised to 1000, so the kernel ends the child, not trss, when the container runs out of memory. trss stays up, parses every feed each cycle, adds the selected items concurrently, serves the screens and previews rules against stored history.

trss reads at most 2 MiB of an RSS feed (real feeds were 50 to 300 KB on 2026-10-01), five feeds at a time, so a feed that is huge or never ends cannot exhaust that memory. A feed whose response announces a longer body is refused before it is read, and any other is dropped as soon as what has arrived passes the cap. The channel counts as unread for that cycle, like one whose server did not answer: the cycle log says `the feed is larger than 2097152 bytes`, and its torrents are not cleaned up (the cap is `MAX_FEED_BYTES` in `crates/trss-collect/src/feed.rs`).

Transmission is limited to 0.5 CPU and 512M, and runs in `transmission.slice`, which caps it softly at 384M (`MemoryHigh`). Most of that memory is page cache from writing downloads. Above the soft cap the kernel reclaims the cache and slows the writer down, and never kills it.

At the hard limit, RHEL 9 kernels from 5.14.0-687.36.1 to 687.53.1 kill `transmission-daemon` while it writes, even when reclaim frees pages (RHEL-211058, fixed in 687.54.1 by RHEL-269060). The container keeps running because s6 restarts the daemon inside it, but every kill loses the progress, renames and labels saved since Transmission last wrote its resume files.

Check for kills on the host:

```sh
grep oom_kill /sys/fs/cgroup/transmission.slice/docker-*.scope/memory.events
```

### Work covers

Each work in the library has one cover: an AniList entry's cover image or a file uploaded in the work's page (tap the cover). trss reaches AniList over HTTPS, so it needs outbound access to `graphql.anilist.co` (searches) and `s4.anilist.co` (cover images); no account or key is involved.

- When the app records a work for the first time (a new folder in a watch folder, or every work of a watch folder that is added), trss searches AniList for the work's folder name and selects an entry only when exactly one entry has that title (ignoring case and spacing) and the search was read to its end. Anything less clear stays empty for the user to choose. A rescan, a restart or opening a page never searches again. Works recorded before this version are not searched for; open the cover and use `자동으로 다시 찾기`.
- The worker sends at most one AniList request every 2 seconds, together with the web's searches, and waits as long as AniList asks when it answers `429`. Adding a watch folder with 500 works therefore takes about 20 minutes of searching in the background; collection and the web go on meanwhile. The queue lives in the database, so a restart continues it.
- Images are judged by their bytes (JPEG, PNG or WebP): the first bytes name the format and the header must state at most 8192 pixels a side and 12 million pixels, in a file of at most 10 MiB. Nothing is decoded (the app stores and serves the bytes as they came), so a file that is broken after its header is accepted and shows as an empty place until it is uploaded again; the memory an image takes is its bytes, not its pixels. Images are stored under new names in `TRSS_DATA_DIR/artwork/`. The web takes in at most two uploads or picked covers at once (10 MiB each, from the first byte to the stored file) and holds at most 32 MiB of cover files being read or sent (a file waits for room, and gives it back when its last byte is sent or the client leaves): 52 MiB at most beside the process's own memory (about 14 MiB a minute after 0.8.0 started, on 2026-10-22), far inside its 384M limit. The worker holds the one cover it fetches (10 MiB). A fetched image that is shorter than the length its response announced is refused, and an upload body that takes more than 60 seconds to arrive is dropped. Images are only fetched from AniList's image host, from addresses AniList's own answers give.
- `TRSS_ANILIST_URL` and `TRSS_ANILIST_IMAGE_ORIGINS` (comma-separated origins) override AniList's addresses, for local testing only.

### Season info

A season's air dates, episode count, studios, genres, original title and synopsis come from the AniList entries linked to the local season (one or more, in order). trss uses the same outbound access as the covers (`graphql.anilist.co`); nothing new is needed.

- When the app records a work (or a first season) for the first time, trss searches AniList for the folder name and links the entry only on the same exact, unique title match and complete search as for covers (up to four search requests, then one request for the entry). The link is marked `자동` and the user can change it. Works and seasons recorded before this version are not searched for; open the season and use `연결 바꾸기`. Later seasons are never linked by the app: the previous season's last entry's sequels are offered and the user confirms one.
- Entries that are not yet released or are releasing are received again once a day by the worker (one request each, at the same pace as the covers, `429` waits included). Finished entries are received again only from `정보 다시 받기`.
- The queue runs on its own, so it never holds up collection or the cover queue. Descriptions are shown as plain text only.

### Anissia schedule

The 구독 tab's `편성표에서 추가` reads the airing schedule (weekdays, `기타`, `신작`) and an anime's subtitle creators from Anissia. trss needs outbound HTTPS access to `api.anissia.net`; no account or key is involved. Times are Asia/Seoul.

- The web asks Anissia when someone looks and keeps each answer for 5 minutes. If Anissia is slow, down or answers something unreadable, the schedule shows the reason with a retry; subscriptions already made keep showing the weekday and time stored with them.
- Every Anissia request, from the web and the worker together, is spaced at least 2 seconds apart through the database, waits as long as Anissia asks after a `429`, reads at most 2 MiB and follows no redirects.
- For each subscribed anime the app stores the schedule values (title, original title, weekday, time, start and end dates, status). trss asks again once a day, in a queue of its own; an anime that cannot be read is retried after an hour and one that Anissia no longer lists keeps its stored values.
- Subtitle candidates: Anissia keeps one line per anime and subtitle creator (the last episode, post address and update time the creator entered) and overwrites it, so trss reads the recent captions list (`/anime/caption/recent/<page>`, pages from 0 to the empty one) every 30 minutes, for every anime whether or not a season is linked, and keeps each state of a line as an observation: creator (an app-made source ID per Anissia anime and creator name), post address, episode text as written, `updDt` as written and as a time (a value without a zone is Asia/Seoul), and when it was first seen. A line that changes episode, address or update time adds an observation; one that vanishes deletes nothing. The reading runs on its own, is not tied to the RSS cycle's interval, shares the 2-second request spacing, and a `429` or a failure is read again at the next 30 minutes with the pages already read kept. Linking a season (from the work detail, or when the worker connects a subscription to its season), and `새로고침`, make the worker read that anime's own lines at once (the `anissia_captions` command), which also reaches lines older than the list's 90 days. `GET /api/library/works/{id}/seasons/{n}/anissia/candidates` gives a linked season's observations.
- `TRSS_ANISSIA_URL` overrides Anissia's address, for local testing only.

### Title waiting

A subscription can be made before an anime's first episode is in the channel (`아직 첫 화 전이에요` in the subscribe flow). It has no match phrase, so it receives nothing. The channel must already have recorded items, so that a title that appears afterwards can be told from the ones that were there. When a work shows up in that channel's history for the first time after the subscription began, and no rule matched or received it, the 구독 tab offers it as a title candidate; the user picks which waiting subscription it belongs to, and the work becomes the rule's match phrase. Items of that work recorded before the choice are past items: the cycle never receives them, and only the ones ticked in the preview are. `거절` hides a work in that channel for good. Candidates are computed when read (`GET /api/subscriptions/candidates`), so a paused or archived subscription offers none and nothing needs cleaning up. The migration adds `rule_subscriptions.titled_at` and the `rejected_titles` table.

### Weekly schedule and the first run

The home screen (`GET /api/schedule/week`) shows this week (Monday to Sunday, Asia/Seoul) from what the app stores: a card for each non-archived subscription, placed by the weekday and time of its stored Anissia snapshot, with the video and subtitle lines taken from the connected season's holdings. Nothing is asked of Anissia or Transmission when it is opened. The episode number of a card comes from AniList's air times of the connected season when it has them, and otherwise is counted in weeks from Anissia's start date, which a break week or a double episode makes wrong from there on.

Two things come from the worker, so they stay empty until trss of this version has run a cycle: the `다음` time of the collection status (the worker records its cycle interval when it starts) and `영상 받는 중` (the cycle records the hashes of the torrents Transmission is downloading, next to the counts, and the episode is read from the title of the item that torrent was received for).

`영상 받는 중` trusts the torrent hashes only while they are fresh: they are ignored once the snapshot is older than three of the worker's cycle intervals (and always when no interval was recorded), so a stopped worker does not leave a card stuck on it. A cycle that has started and not ended is running until it has gone on longer than the larger of 30 minutes and ten intervals: while it runs, the torrent hashes it found before it are not dropped, however short `TRSS_WORKER_INTERVAL_SECS` is.

A card leaves once Anissia's end date for the anime has passed. An anime without an end date leaves when the worker's daily refresh has asked every week of Anissia's schedule, all of them answered with something listed, and none listed the anime (`anissia_anime.unlisted_at`; listing it again clears it). A rule's archive removes the card at once.

A weekday whose list comes back empty while an anime's snapshot was last listed in it is not believed: that refresh does not record those anime as unlisted, and the other weekdays decide theirs as usual. A refresh that failed, was refused (`429`) or stopped halfway never records that, so while Anissia cannot be reached the cards stay.

An anime Anissia marks `OFF` (no broadcast) keeps its card with a quiet `결방` line and no episode number; a paused subscription still shows `받기 멈춤`.

The collection is taken to have stopped when an unfinished cycle has gone on longer than the larger of 30 minutes and ten intervals, or when a finished cycle's next check is more than one cycle interval overdue with no cycle started since; the folder readings and the commands run apart from the cycles and do not count. Then (`cycle.stalled` of `GET /api/collect/status`) the collection status here and on the collect screen says `RSS 확인이 멈췄어요. 컨테이너 로그를 확인해 주세요.` in place of a past time (`RSS 멈춤` on a phone's summary line).

Times in the collection status (here and on the collect screen) read as `오늘 17:53` / `어제` / `내일` / the date, always in Asia/Seoul.

A new install shows the `처음 설정` checklist instead. Migration 21 decides at upgrade time whether the install began empty (no channel and no registered watch folder); only then is there a first run, so an existing install never sees the checklist. A step is done when it has happened: a watch folder was registered (kept by a trigger on `watch_folders`, so removing the folder later does not undo it) or an import was applied (channels that exist for another reason do not count). Skips and the end of the checklist are kept in the database so every device shows the same. Once both steps are done or skipped the checklist has ended for good: removing folders or channels afterwards does not bring it back, and only taking back the last skip does (the web offers that in the notice right after the checklist disappears). Migration 22 gives an install that was already past the checklist its end. Importing the legacy YAML on an install with no channels asks nothing per channel. Nothing needs configuring.

### Season link

trss connects a subscription to the library season its videos appeared in. After each library scan it takes the history items a subscription's rule received, finds those torrents' files in Transmission, and looks for the work and `Season NN` folder holding that video. Folder names are never compared, so a person's own files or another rule's videos connect nothing, and a rule whose videos appear in more than one season stays unconnected. A connected season stays. If another Anissia anime already holds that season the rule is not connected, and its detail says which anime holds it. The link needs the torrent to still be in Transmission when the video first shows up in the library. A file is matched by the path Transmission reports (its download folder plus the file's name), with no mapping, so Transmission and the worker must see the download folder at the same path; when a rule's torrents are in Transmission but none of their videos is in the library at that path, the worker logs the rule's ID once per start. A rule that stays unconnected is tried again only when it receives another torrent or the library changes.

### Episode offset

A rule's `episode` turns a release's number into the number in the video's name. For a subscription whose rule has not picked anything yet, trss looks at the lowest whole episode among the items its cycle is about to pick, before they are sent to Transmission. The season is the one the subscription is connected to, or, before any video exists to connect it from, the one named by the rule's save folder (`<work>/Season NN` under the collect folder, read the way the renamer reads it). The earlier seasons' episodes are the sum of the AniList counts linked to the work's earlier seasons, as [Season info](#season-info) describes; a missing season folder, link or count makes the sum unknown and nothing is guessed.

When the first release is that sum plus one, the offset is set to minus the sum (`- 25` after 24 episodes gives `-24`, so the first video is `S03E01`; `- 49` after 48 gives `-48` even when the field still holds the previous season's `-24`) and the rule's detail shows `자동` with the grounds and the value it replaced. This happens only before the rule has picked an item, whatever the field holds, and only once per rule: a value changed by hand or undone is never set again.

A rule that has picked items gets a suggestion instead, shown whenever the suggested value differs from the field. What it suggests depends on the case:

- A first release one past the episodes of the last few seasons (`- 25` for a third season after two of 24) is suggested as numbering that runs on from those seasons, never applied by itself.
- A split cour that starts again at `- 01` in a season folder holding all the earlier cours' episodes is suggested a positive offset, the episode `- 01` becomes (`13` after a 12-episode cour).
- In every other case the rule is received without conversion and the detail says why it cannot suggest a value.

`되돌리기` beside the grounds restores the earlier value and renames the videos received under the app's value (`S03E01` → `S03E25`): through Transmission when the torrent is finished and still there (so it keeps seeding), on disk without overwriting when it is gone. A file whose new name is taken, whose torrent is still downloading, or whose revision replacement is under way is left with its reason, and `이어서 되돌리기` continues from the remaining files.

Nothing here changes the subtitle episode mapping.

### Watch folders and inotify

trss watches the watch folders (including the collect and archive folders) with inotify. A new episode shows up in the library a few seconds after its file appears, without waiting for the 5-minute cycle, and only the work that changed is read again. Changes made by other containers on the same host (Transmission) are seen the same way. The cycle still reads a folder's directories by their modification times when the worker starts, when the kernel reports that it dropped events, for directories that could not be watched, and once an hour as a safety net (for example for a share that other machines change over SMB or NFS, which the kernel does not report).

Each watched directory (a watch folder, each work folder, each `Season NN` folder) takes one watch of the kernel's `fs.inotify.max_user_watches`, about 1 KB of kernel memory, counted per user for every process of that user on the host. A library of about 1,500 folders uses about 1,500 watches; the worker also opens one inotify instance per watch folder (`fs.inotify.max_user_instances`, 128 by default).

When the limit is reached the watch folder's row in the settings says how many directories are not watched and why; the worker then checks those works itself every cycle, and nothing else changes.

To watch them as well, raise the limit on the host:

Not tested:

- The limit taking effect on a RHEL 9 host: the stand-in container may not change kernel settings (2026-10-22).

```sh
echo 'fs.inotify.max_user_watches=524288' | sudo tee /etc/sysctl.d/60-inotify.conf &&
sudo sysctl --system
```

<details>
<summary>Tested runs</summary>

Tested:

- The block as pasted into bash in an unprivileged `archlinux` container, with `sudo` replaced by a script that runs the command directly: it wrote the line to `/etc/sysctl.d/60-inotify.conf`, and `sysctl --system` applied that file and was refused the key (2026-10-22).

</details>

Read the current value on the host:

```sh
cat /proc/sys/fs/inotify/max_user_watches
```

To see how many watches the worker holds, look for the `directories watched` line it logs when it starts.

### Channels and the collect folder

Channels and rules live in the app database and are edited in the web. To bring over a channel configuration YAML from versions up to 0.3.x, which ran as a cron job and read it from `CHANNELS_CONFIG_URL`, use Settings → Data → Import in the web. Importing only writes channels and rules (and the collect folder, below) and the subscriptions you check; it adds, renames and removes nothing.

The review step also reads the two comment lines directly above each rule (no blank line between) and suggests a subscription for that rule:

```yaml
# Mon. 23:30. <creator name>
# https://anissia.net/anime?animeNo=<number>
```

The first line is the weekday (`Mon` `Tue` `Wed` `Thu` `Fri` `Sat` `Sun`), the time (`HH:MM`) and the subtitle creator, separated by `. `; the creator is everything after the second separator. The second line is the Anissia address. A suggestion with a creator starts checked. A line 1 that does not fit (or has no creator) leaves an address-only suggestion, unchecked and shown as `제작자 미정`, and a comment with no readable Anissia address shows why it could not be read and imports the rule alone. Checked suggestions become subscriptions in the same import, with the weekday and time Anissia gives (the comment's own weekday and time are shown only while Anissia cannot be reached; the daily refresh fills in the stored ones). The import receives nothing, and the items already in the history count as past ones. A channel the import creates has no history yet, so a subscription leaves everything its feed holds at the channel's first read as past ones too (shown as `first_read` in the rule's 지난 회차, for the user to pick); plain rules receive the matching items of that first read as ever. The grammar is in `crates/trss-import/src/comments.rs`; adjust it there if your comments are written differently.

A channel has no folder of its own. Every torrent is saved under the app's **collect folder** (Settings → Collection → Collect folder) plus its rule's save folder, so a rule with the save folder `Show/Season 01` saves to `<collect folder>/Show/Season 01`. The settings screen also takes an optional **archive folder**, where the work folders of archived rules go (below).

- Write both folders as paths the trss container sees (they start with `/downloads`, the mount above). The web checks that they exist, that neither is the other or inside the other, and that they are on the same filesystem. It cannot check that Transmission can write to them; that shows up when it tries.
- Until a collect folder is set the worker adds nothing and records no failure for the items a rule picked. The status board says so, and the next cycle after the folder is set receives those items. A new database starts without one; importing a YAML file sets it (the channels' shared folder, or their common parent), and so does an upgrade from a version with per-channel folders.
- Upgrading from 0.4.x (channels with their own base folder): the migration makes the shared base folder the collect folder, or, when the channels' base folders differ, their common parent, and puts the remainder in front of each rule's save folder. Every rule keeps saving to exactly the folder it had. The old `base_dir` column is dropped, so a database opened by this version cannot go back to an older one; keep a copy of `trss.db` before upgrading. Relative base folders with nothing in common cannot be folded into one collect folder; the upgrade then refuses to start and leaves the database unchanged.
- Importing a YAML file whose channel folder is outside the collect folder reports that channel with the reason in the review step and does not import it.

### Archiving and restoring a rule

A rule's **work folder** is the first part of its save folder: `Clevatess` for `Clevatess/Season 02`. Archiving a rule (the rule's `보관` button) turns it off and then moves its work folder, `.trss/` included, from the collect folder into the archive folder; restoring it (`복원`) moves the folder back and only then turns the rule on. The web only accepts the request; the worker part of trss carries it out at once, beside a cycle or other commands, and the rule shows how the move went. While the folder moves, nothing else is added to it, renamed in it or read from it: a cycle's new episode for the rule waits for the move and is then left alone, as the rule is archived.

- Transmission's torrents inside the work folder (judged by their folder's text and by where it really is, links followed) are moved first, with `torrent-set-location` (files moved), and the worker waits until Transmission reports the new folder. Then the worker renames what is left. It only renames within one filesystem, with `RENAME_NOREPLACE` (a filesystem without it is refused), so moved files and folders keep their owner; the folders Transmission makes for its torrents belong to Transmission's user. What the worker itself creates in the media belongs to the user it runs as, 1000:1000 (see [Running as 1000:1000](#running-as-10001000)).
- Nothing moves while one of those torrents is not finished (downloading, verifying, or a magnet still fetching its metadata; finish it or remove it in Transmission, then move again), reports a local error, has a download folder written with `..` or reached through a link into or out of the work folder, or has a file the destination already holds. Files found only at the destination count as the torrent's own (moved there by hand) only when none is left at the source and each has the torrent's size. A move error Transmission reports ends the move with its text. When Transmission takes longer than the wait (5 minutes), the request stays open and the worker checks again a few seconds later, up to five times.
- When the archive folder already has the work folder (an earlier season archived before), the two are merged, season folders file by file. If any file is at the same place on both sides, nothing moves and the rule lists those files; clear one side and use `다시 옮기기`.
- The folder stays where it is when no archive folder is set, when the rule saves into the collect folder itself, or while another active rule (in any channel) still saves into the same work folder. It moves when the last of those rules is archived. A work folder that is a link, or that holds a link leading out of the two folders, is not moved.
- Where a work folder is, is read from the disk each time. A move cut short (the worker stopped or restarted) is picked up again by the worker, which moves what is left. A folder moved by hand is recognised too.
- While a rule's folder is moving, the rule's save folder cannot be changed, and no rule can be created in or moved into that work folder. A save folder with `..` is refused when it is typed (rules saved before keep working).

### Archive suggestions

The 구독 tab (beside the title candidates) and a rule's detail suggest archiving a rule that is not archived and has a match phrase (a subscription still waiting for its title has none), paused rules included. Nothing is switched off by itself, and the menu badge does not count suggestions. A suggestion is computed when read (`GET /api/archive-suggestions`, which the future 할 일 list reads too) from the stored Anissia snapshots and the history; nothing is asked of Anissia or the feed, so while Anissia cannot be reached the refresh finds nothing out and no `방영 종료` ground appears. Grounds:

- `방영 종료`: the subscription's anime has an end date that has passed (the weekly schedule drops its card by the same rule), or it has none and the daily refresh found it no longer listed.
- `새 항목 없음`: 4 weeks (exactly) have passed with no new item matching the rule. A *new item* is one the channel first recorded in the last 4 weeks whose title the rule matches (the channel's excludes and the rules ahead of it set aside as the rule preview does), whatever became of it; so what an earlier rule took, or a paused rule saw, counts. The 4 weeks count from the rule's last receive, and for a rule that never received from when it started collecting: the latest of when the app first had the rule, when it became a subscription or was given its title, when it was last turned back on or restored, and, for a rule from before the migration, when its channel was first read. A channel that recorded more than 20,000 items in 4 weeks gives its rules no `새 항목 없음` ground (the web logs it), and so does a rule whose regular expression does not compile.

`수집 유지` remembers the grounds it saw and nothing else changes about the rule: the same ground does not suggest it again (the end of the anime is told by its end date, the quiet stretch by the moment it counts from), and a new ground does (an ending, or a quiet stretch after another receive). Archiving from the suggestions sends each rule's own `rule_archive` command in the list's order, one after another, so a work folder shared by several of them moves once, with the last. The migration adds `rule_started` (stamped by triggers when a rule is created; rules from before it have no stamp) and `archive_suggestion_kept`.

### Video revisions

When a feed brings a higher revision of a release whose episode is already in the rule's folder (`[SubsPlease] Show - 14v2 (1080p) [1A2B3C4D].mkv` after `… - 14 …`), trss receives it under its own name, checks that its CRC32 is the one in the name, and only then removes the old video and gives the new one the episode name. The work's episode row then shows a quiet `v1 › v2` line, so a subtitle timed for the old video can be checked.

#### The old video is deleted, for good

If it is the only file of a torrent trss added, that torrent is removed from Transmission together with its data; if no torrent holds it, the file is deleted. There is no copy to restore.

A video in a torrent of several files (a batch), in a torrent trss did not add, or named by two torrents at once is never touched: the replacement fails and both files stay.

Which torrent holds a file is read from Transmission's file lists by path and confirmed on the file itself (same device and inode), so a folder Transmission names through a symbolic link still matches; Transmission and the worker must still see the media at the same path (`/downloads`), as for [Season link](#season-link).

#### What is deleted is checked again just before

The worker looks at the episode's file again right before removing it: a torrent's file must be a lower revision of the same release, and a file of no torrent must still have the CRC32 it had when the revision was decided (it is read again for that).

Then, immediately before the torrent is removed or the file deleted, the path must still be the file that was checked (same device, inode, size, modification time and status-change time, so a rewrite that puts the modification time back is caught too); a file replaced or changed in between, or while its CRC32 was read, is not deleted, and a file that is gone by then counts as removed. Anything else fails the replacement and keeps the file. A video inside a torrent's own folder is never replaced, since removing that torrent would take the folder.

#### One revision at a time, never downwards

When `14v2` and `14v3` are both on their way, only one replacement removes the episode's video at a time, the lower one is skipped once the higher one is in place or on its way (its video stays under its received name), and a lower revision that appears later is recorded as a duplicate and not received.

If the higher one then fails (`받기 실패`), the lower one skipped for it goes back to its first step on a later cycle and replaces the old video after all, with the same checks; the higher one stays a failure with `다시 받기`, and once received it replaces the lower one the same way.

The same release reaching the folder through two channels is one torrent and one replacement, and once its old video is replaced, neither channel's old item is received again.

#### A failure before the old video is removed keeps it

The failures that keep it are a CRC32 that differs, an old torrent that could not be removed, and an old file that could not be deleted. The episode row shows `받기 실패` with both files and why, and so does `GET /api/todo/receive-failures`; it goes away once one of the two files is deleted.

A failure before the new video was received (its torrent stopped, reports a local error, or sits outside the rule's folder) is looked at again every cycle: once the torrent is right the replacement goes on, and a cycle that still sees the item in its feed adds it again when the torrent is gone.

After the old video is removed, the worker renames the new one every cycle until the episode name is free and the rename goes through; it never renames over a file. While the rule's folder itself is not there (a mount that is away), no step of a replacement decides anything from files missing in it; it waits for the next cycle. A torrent removal Transmission did not answer is looked at again (it may have happened), and an old torrent removed while its file stayed waits, shown as `받기 실패`, until that file is gone.

Before the old video is looked at and again right before it is removed, the worker also checks that the new video is still the one whose CRC32 it read: the same device, inode, size and times under its received name, or, when those changed (a remount, a copy), the same CRC32 read again. If not, it removes nothing and waits, shown as `받기 실패`.

If the new video is deleted instead, goes missing after the old video was removed and before it took the episode name, or is not the checked file when the old video is to go, the replacement ends once two looks in a row (one per cycle) find it so, with the folder there both times and no look between them finding the video. What follows depends on when it ended:

- One that ended after the old video was removed leaves the episode with no video, so it stays listed as `받기 실패` (old video removed, new video missing) until a video is under the episode name again.
- One that ended with the old video in place is not shown (one look alone may be a mount that is away for a moment): nothing is deleted or renamed, the old release is not received again, later revisions of the episode no longer wait for it, and a lower revision skipped while it was on its way starts over as when a higher one fails (an ended replacement never put its video in place, so it holds no lower revision back), with no need to put the file back.

#### `다시 받기` for a download that stopped

When the revision's torrent left Transmission or reported an error before the video was received, the episode row (and the failure source) offers `다시 받기`, also after the release left the feed or when it came from a past episode search. It adds the torrent again from the link recorded in history, never one sent by the browser (or starts the one Transmission still has) and the replacement starts over with the same checks; the old video stays until the new one is checked.

It is not offered while the rule is paused or archived, once a higher revision of the episode is in place or on its way, or when the rule's folder is no longer the one the replacement was decided for (a torrent added now would land away from the old video).

When the worker runs a `다시 받기` of a revision (this one, or a `버전 미상` one), it also looks at the video under the episode name the way a cycle's decision does (its torrent and history, or else its CRC32) and refuses, saying why in the command, when that video is the same or a higher revision already, also one that came in as an ordinary item because the episode name was free. The web already hides `다시 받기` when the episode name holds a file and a finished replacement or a received torrent the worker last listed (within 30 minutes) is the same or a higher revision; the CRC32 case only the worker can tell. A stopped replacement refused this way ends as skipped and leaves the failure list.

#### Nothing ever takes a name that is taken

Transmission's rename leaves the file where it is and still answers `success` when the new name exists, so every rename (the episode name after an add, `다시 받기`, a replacement) is now skipped when that name is already in the folder; the video then keeps the name it was received under. This applies to the old binary too.

#### A revision is named as its episode

Every rename derives the episode name from the release name without its revision marker (`06v2` → `06`): `trname` reads Erai-raws' `[Erai-raws] Show - 06v2 [1080p CR WEBRip HEVC AAC][MultiSub][1BBD34E6].mkv` as another episode, taken from the CRC32 bracket.

The revision marker is the last `NvM` before the CRC32 bracket, so a show named `Show 3v3` keeps its `3v3`; a `NvM` followed by a bare number outside brackets is part of the name too, so `Show 3v3 - 06` is the first revision of episode 6. History keeps the release's own title.

#### A revision that is not received on its own

A revision whose name has no CRC32, or whose existing video matches none of the release's revisions, is not received on its own: it shows as `버전 미상` in the collect history, and `다시 받기` receives it and replaces the old video (without the CRC32 check when the name has none). Another group's release of the same episode is no revision and is received as before, under its own name.

#### CRC32 cost

The received file is read once, in 1 MiB blocks, when its download has finished: memory stays at that block whatever the file's size, but the worker reads the whole file from the disk (about 1.5 GB for a 24-minute 1080p episode), which takes seconds to tens of seconds on a hard disk and delays the rest of that cycle.

A video already in the folder whose torrent history does not know is read the same way when its revision first appears, and once more right before it is deleted. Only one file is read at a time. The outcome is stored, so a decision is not read again.

#### Transmission's whole file list

Transmission's whole file list (every torrent with its files) is asked for at most once per cycle for the decisions, and only when a revision's episode file is not its own torrent's; the replacement steps ask for it again only after they changed Transmission.

#### Stored progress

Each replacement's progress is stored (migration 25, the `video_revisions` table), and every step is written before it acts, so a worker stopped halfway carries on from what is on disk when it starts again; a step that acted but was not written (the rename went through, say) is recognized from the disk on the next look.

A revision's decision is written in one transaction with its item's history, so an item is never recorded as received or `버전 미상` without it.

### Past episode search

A rule's detail has a `지난 회차 검색` section for episodes the feed no longer holds: confirm the query and the release range, preview the judged results, then receive the ones you tick. The search is one read of the tracker's search RSS (never the HTML search page) made by trss for that one search; it creates no channel, and a search you leave or finish keeps nothing, so the history holds only what was received. A search is kept in trss's memory for 30 minutes (at most 6 at once; a new search of a rule ends its old one), so restarting trss ends running searches. The picked results are sent by key and trss adds them; the browser never sends a link.

- **Query.** The channel's search format (`[SubsPlease] {match} 1080p`, set in the channel tab) with the rule's match phrase put where `{match}` is. A channel without a format starts from the match phrase, and you edit the query in place. A regular-expression rule and a rule that is still waiting for its title have no phrase to search for, so they start with an empty box and a sentence saying why. The URL is the channel's own with `q` set to the query (and `p` dropped); the channel's other query values (category, filter, token) are kept.
- **Pace.** Requests to one host are at least 3 seconds apart, however many searches ask: the next allowed time of each host is a row of `search_pace` (migration 26), and each request takes its slot in one write transaction before it is sent. Only search requests take slots, and only the web sends them: the worker does not use the row, and its cycle reads of the feed are unchanged. A request waits for its slot at most 60 seconds; when the slot is further away (the host is blocked, or the clock moved), the search fails at once and says when to try again. A `429` ends the search with the tracker's `Retry-After` and blocks that host's slots until then, and a request that was waiting for its slot is not sent into the block.
- **Long series.** The tracker returns at most 75 results. When the first page is full, the episodes of the range that are still missing are searched in groups of 10, `{series} - (1000|1001|…)`, using the numbering of the first page's real titles, and merged without repeats. At most 20 extra searches are sent (so a search takes up to a minute), a range spans at most 2000 episodes, a search keeps at most 2000 results, and at most 20 existing videos are read for their CRC32.
- **What is selected.** Only an episode that is missing from the work folder and that no torrent of the rule or channel in Transmission holds. A higher revision of an existing episode and a result whose version cannot be told (`버전 미상`) are shown but not selected; tick one to receive it with the checks of video revisions above. Batches, episodes already in the folder and episodes of another season are never selected, and results outside the range are folded into a count. Whether an episode was received from another release is judged from the recorded history and the real files in the work folder.
- **Received once, gone now.** An episode the history shows as received is missing again, and its result can be ticked, when its video is not in the work folder and its torrent is no longer in Transmission. Only a single episode is judged so (a batch stays received), and only when the work folder exists and was read whole (at most 5000 entries): an unmounted volume or a very large folder holds every received episode, as does another torrent of the rule for the same episode that Transmission still has. The search does not call Transmission: each cycle leaves the hashes of all its torrents (migration 27, `transmission_listing` and `transmission_torrents`), and a search compares them with the torrent hash in history. Without a list yet, for an item received after the list was taken, or for an item whose hash history does not know, the torrent counts as still there. When such a result is received, trss asks Transmission and the folder again and adds it only if both are still empty, so a list that has gone stale never adds a duplicate.

## Local development environment

`dev/compose.sh` runs the same two compose files on a development machine, with an image built from the working tree and a copy of a server's data. Its data lives in `dev/local/`, which Git ignores; its settings are `dev/dev.env`.

| Command | What it does |
| --- | --- |
| `dev/compose.sh pull` | Copies the server's data, once; `--force` replaces it |
| `dev/compose.sh up` | Builds the images and starts Transmission, `trss-browser` and `trss` |
| `dev/compose.sh logs` | Follows `trss`; `dev/compose.sh logs trss-browser` follows the browser |
| `dev/compose.sh down` | Stops them; `dev/local` stays for the next `up` |

- `up` builds with cargo's caches: the crates.io registry, the git checkouts and `target/` are BuildKit cache mounts of the local Docker builder, shared by `Dockerfile` and `Dockerfile.browser` and by every checkout and worktree that builds on the machine. The first build compiles every dependency; after a change only the workspace crates whose files changed and the crates that depend on them compile, and the binaries relink. Cargo tells there which files changed by their content, not their times (`deploy/stamp-sources.sh`), so a build never takes binaries that another worktree's sources produced.
- `pull` reaches the server over ssh (`TRSS_DEV_SERVER`, default `j4105`; `TRSS_DEV_SERVER_DIR`, the compose folder under the remote home, default `trss`). It stops the server's trss containers (`trss`, or `trss-worker` and `trss-web` on a server that runs release 0.7.0 or before) for the few seconds the copy of `/data` takes, so the database is one consistent state, and starts them again. It copies the database, `artwork/` and `subtitle-files/`, and only the names of the files under `/downloads`: it recreates them as empty files under `dev/local/media`. Reading a watch folder looks at names and folder metadata only, so the library reads as it does on the server, without the videos.
- The local Transmission starts from `dev/transmission-settings.json`: added torrents stay stopped, and DHT, PEX, local peer discovery and port forwarding are off. A command that starts a torrent still moves at most 1 KiB/s (`SPEED_LIMIT_UP`/`SPEED_LIMIT_DOWN` in `dev/dev.env`).
- trss listens on `127.0.0.1:8080` only. The feeds, AniList and Anissia are the real ones, so a cycle reads the server's channels and new work folders fetch real covers.
- The containers run as 1000:1000 here too, so `dev/local/data` must belong to that user (`prepare` makes it as you, which is 1000 on most Linux machines; another user sets `TRSS_UID` and `TRSS_GID` in `dev/dev.env`). A `dev/local/data` that an earlier run as root left files in is refused at the start; `chown -R` it as [Running as 1000:1000](#running-as-10001000) says.
- Changes stay in `dev/local`: archiving a rule moves the empty files there, and the server's data is never written.

To free the space of the build caches (a few GiB) or start over:

```sh
docker builder prune --filter 'description~="with id \"/trss-"'
```

### Tests

Run the tests on this machine's glibc:

```sh
cargo test --workspace -j 4
```

This does not build `crates/trss/tests/it/remote_screen_docker.rs`, the remote screen against the real server browser, which needs Docker and the browser image (`TRSS_BROWSER_IMAGE`, by default `ghcr.io/syrflover/trss-browser:local`, which `dev/compose.sh up` builds). Its feature builds it, and its tests are ignored until asked for:

```sh
cargo test -p trss --features remote-screen-docker --test it remote_screen_docker -- --ignored --nocapture
```

### Releasing

A release is a tag: pushing one makes the [Deploy workflow](.github/workflows/deploy.yml) build and publish the app image and the browser image under that tag and `latest`. Before tagging, run the workspace tests on the release's target too:

```sh
dev/musl-test.sh
```

Arguments after the script name go to `cargo test`. Then update the server as [Run, stop, update](#run-stop-update) says.

<details>
<summary>Why the tests run on musl, and what the script does</summary>

- The release binaries are built for `x86_64-unknown-linux-musl` in `clux/muslrust:stable` (`Dockerfile`), while the usual `cargo test` runs on the development machine's glibc. musl's standard library differs in what it can read (it gives no file birth time, for one), and the bundled C code (SQLite, liblzma) builds differently, so a test can pass on one and fail on the other. The script runs `cargo test --locked --workspace -j 4` in that image.
- It runs the tests as you, not root (some tests expect a write to be refused), with a tmpfs `/tmp` as on the development machine (the tests' SQLite files sync to it), builds into `target/musl`, and uses this machine's cargo registry and git checkouts. Tests marked `#[ignore]` are skipped, as in the usual run.

</details>
