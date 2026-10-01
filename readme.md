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
- `TRSS_DATA_DIR` holds `trss.db`, the worker's lock files `trss.db.worker.lock`, `trss.db.artwork.lock`, `trss.db.seasons.lock` and `trss.db.anissia.lock`, and `artwork/`, the work covers (see [Work covers](#work-covers)). Keep it on a local disk, not an SMB or NFS share: SQLite and the locks rely on local file locking.
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

The worker reads at most 2 MiB of an RSS feed (real feeds are 50 to 300 KB), five feeds at a time, so a feed that is huge or never ends cannot exhaust that memory. A feed whose response announces a longer body is refused before it is read, and any other is dropped as soon as what has arrived passes the cap. The channel counts as unread for that cycle, like one whose server did not answer: the cycle log says `the feed is larger than 2097152 bytes`, and its torrents are not cleaned up (the cap is `MAX_FEED_BYTES` in `src/worker/feed.rs`).

Transmission is limited to 0.5 CPU and 512M, and runs in `transmission.slice`, which caps it softly at 384M (`MemoryHigh`). Most of that memory is page cache from writing downloads. Above the soft cap the kernel reclaims the cache and slows the writer down, and never kills it. At the hard limit, RHEL 9 kernels from 5.14.0-687.41.1 kill `transmission-daemon` while it writes, even when reclaim frees pages (RHEL-211058, reverted in RHEL-255363). The container keeps running because s6 restarts the daemon inside it, but every kill loses the progress, renames and labels saved since Transmission last wrote its resume files. Check for kills with `grep oom_kill /sys/fs/cgroup/transmission.slice/docker-*.scope/memory.events`.

### Work covers

Each work in the library has one cover: an AniList entry's cover image or a file uploaded in the work's page (tap the cover). Both containers reach AniList over HTTPS, so they need outbound access to `graphql.anilist.co` (searches) and `s4.anilist.co` (cover images); no account or key is involved.

- When the app records a work for the first time (a new folder in a watch folder, or every work of a watch folder that is added), `trss-worker` searches AniList for the work's folder name and selects an entry only when exactly one entry has that title (ignoring case and spacing) and the search was read to its end. Anything less clear stays empty for the user to choose. A rescan, a restart or opening a page never searches again. Works recorded before this version are not searched for; open the cover and use `자동으로 다시 찾기`.
- The worker sends at most one AniList request every 2 seconds, together with the web's searches, and waits as long as AniList asks when it answers `429`. Adding a watch folder with 500 works therefore takes about 20 minutes of searching in the background; collection and the web go on meanwhile. The queue lives in the database, so a restart continues it.
- Images are judged by their bytes (JPEG, PNG or WebP): the first bytes name the format and the header must state at most 8192 pixels a side and 12 million pixels, in a file of at most 10 MiB. Nothing is decoded (the app stores and serves the bytes as they came), so a file that is broken after its header is accepted and shows as an empty place until it is uploaded again; the memory an image takes is its bytes, not its pixels. Images are stored under new names in `TRSS_DATA_DIR/artwork/`. The web takes in at most two uploads or picked covers at once (10 MiB each, from the first byte to the stored file) and holds at most 32 MiB of cover files being read or sent (a file waits for room, and gives it back when its last byte is sent or the client leaves): 52 MiB at most beside the process's own memory (about 14 MiB idle), far inside its 128M limit. The worker holds the one cover it fetches (10 MiB). A fetched image that is shorter than the length its response announced is refused, and an upload body that takes more than 60 seconds to arrive is dropped. Images are only fetched from AniList's image host, from addresses AniList's own answers give.
- `TRSS_ANILIST_URL` and `TRSS_ANILIST_IMAGE_ORIGINS` (comma-separated origins) override AniList's addresses, for local testing only.

### Season info

A season's air dates, episode count, studios, genres, original title and synopsis come from the AniList entries linked to the local season (one or more, in order). Both containers use the same outbound access as the covers (`graphql.anilist.co`); nothing new is needed.

- When the app records a work (or a first season) for the first time, `trss-worker` searches AniList for the folder name and links the entry only on the same exact, unique title match and complete search as for covers (up to four search requests, then one request for the entry). The link is marked `자동` and the user can change it. Works and seasons recorded before this version are not searched for; open the season and use `연결 바꾸기`. Later seasons are never linked by the app: the previous season's last entry's sequels are offered and the user confirms one.
- Entries that are not yet released or are releasing are received again once a day by the worker (one request each, at the same pace as the covers, `429` waits included). Finished entries are received again only from `정보 다시 받기`.
- The queue takes its own lock, `trss.db.seasons.lock`, so it never holds up collection or the cover queue. Descriptions are shown as plain text only.

### Anissia schedule

The 구독 tab's `편성표에서 추가` reads the airing schedule (weekdays, `기타`, `신작`) and an anime's subtitle creators from Anissia. Both containers need outbound HTTPS access to `api.anissia.net`; no account or key is involved. Times are Asia/Seoul.

- The web asks Anissia when someone looks and keeps each answer for 5 minutes. If Anissia is slow, down or answers something unreadable, the schedule shows the reason with a retry; subscriptions already made keep showing the weekday and time stored with them.
- Every Anissia request, from the web and the worker together, is spaced at least 2 seconds apart through the database, waits as long as Anissia asks after a `429`, reads at most 2 MiB and follows no redirects.
- For each subscribed anime the app stores the schedule values (title, original title, weekday, time, start and end dates, status). `trss-worker` asks again once a day, under its own lock, `trss.db.anissia.lock`; an anime that cannot be read is retried after an hour and one that Anissia no longer lists keeps its stored values.
- `TRSS_ANISSIA_URL` overrides Anissia's address, for local testing only.

### Title waiting

A subscription can be made before an anime's first episode is in the channel (`아직 첫 화 전이에요` in the subscribe flow). It has no match phrase, so it receives nothing. The channel must already have recorded items, so that a title that appears afterwards can be told from the ones that were there. When a work shows up in that channel's history for the first time after the subscription began, and no rule matched or received it, the 구독 tab offers it as a title candidate; the user picks which waiting subscription it belongs to, and the work becomes the rule's match phrase. Items of that work recorded before the choice are past items: the cycle never receives them, and only the ones ticked in the preview are. `거절` hides a work in that channel for good. Candidates are computed when read (`GET /api/subscriptions/candidates`), so a paused or archived subscription offers none and nothing needs cleaning up. The migration adds `rule_subscriptions.titled_at` and the `rejected_titles` table.

### Weekly schedule and the first run

The home screen (`GET /api/schedule/week`) shows this week (Monday to Sunday, Asia/Seoul) from what the app stores: a card for each non-archived subscription, placed by the weekday and time of its stored Anissia snapshot, with the video and subtitle lines taken from the connected season's holdings. Nothing is asked of Anissia or Transmission when it is opened. Two things come from the worker, so they stay empty until `trss-worker` of this version has run a cycle: the `다음` time of the collection status (the worker records its cycle interval when it starts) and `영상 받는 중` (the cycle records the hashes of the torrents Transmission is downloading, next to the counts, and the episode is read from the title of the item that torrent was received for). A card leaves once Anissia's end date for the anime has passed. An anime without an end date leaves when the worker's daily refresh has asked every week of Anissia's schedule, all of them answered with something listed, and none listed the anime (`anissia_anime.unlisted_at`; listing it again clears it). A refresh that failed, was refused (`429`) or stopped halfway never records that, so while Anissia cannot be reached the cards stay. A rule's archive removes the card at once. An anime Anissia marks `OFF` (no broadcast) keeps its card with a quiet `결방` line and no episode number; a paused subscription still shows `받기 멈춤`. `영상 받는 중` trusts the torrent hashes only while they are fresh: they are ignored once the snapshot is older than three of the worker's cycle intervals (and always when no interval was recorded), so a stopped worker does not leave a card stuck on it. With a short `TRSS_WORKER_INTERVAL_SECS`, a cycle that runs longer than two intervals shows the stopped line, and one near three intervals can drop `영상 받는 중` until it finishes; at the default five minutes that takes a cycle of over ten. When the next check is more than one cycle interval overdue (`cycle.stalled` of `GET /api/collect/status`), the collection status here and on the collect screen says `RSS 확인이 멈췄어요. worker가 돌고 있는지 확인해 주세요.` in place of a past time (`RSS 멈춤` on a phone's summary line). Times in the collection status (here and on the collect screen) read as `오늘 17:53` / `어제` / `내일` / the date, always in Asia/Seoul. The episode number of a card comes from AniList's air times of the connected season when it has them, and otherwise is counted in weeks from Anissia's start date, which a break week or a double episode makes wrong from there on.

A new install shows the `처음 설정` checklist instead. Migration 21 decides at upgrade time whether the install began empty (no channel and no registered watch folder); only then is there a first run, so an existing install never sees the checklist. A step is done when it has happened: a watch folder was registered (kept by a trigger on `watch_folders`, so removing the folder later does not undo it) or an import was applied (channels that exist for another reason do not count). Skips and the end of the checklist are kept in the database so every device shows the same. Once both steps are done or skipped the checklist has ended for good: removing folders or channels afterwards does not bring it back, and only taking back the last skip does (the web offers that in the notice right after the checklist disappears). Migration 22 gives an install that was already past the checklist its end. Importing the legacy YAML on an install with no channels asks nothing per channel. Nothing needs configuring.

### Season link

`trss-worker` connects a subscription to the library season its videos appeared in. After each library scan it takes the history items a subscription's rule received, finds those torrents' files in Transmission, and looks for the work and `Season NN` folder holding that video. Folder names are never compared, so a person's own files or another rule's videos connect nothing, and a rule whose videos appear in more than one season stays unconnected. A connected season stays. If another Anissia anime already holds that season the rule is not connected, and its detail says which anime holds it. The link needs the torrent to still be in Transmission when the video first shows up in the library. A file is matched by the path Transmission reports (its download folder plus the file's name), with no mapping, so Transmission and the worker must see the download folder at the same path; when a rule's torrents are in Transmission but none of their videos is in the library at that path, the worker logs the rule's ID once per start. A rule that stays unconnected is tried again only when it receives another torrent or the library changes.

### Episode offset

A rule's `episode` turns a release's number into the number in the video's name. For a subscription whose rule has not picked anything yet, `trss-worker` looks at the lowest whole episode among the items its cycle is about to pick, before they are sent to Transmission. The season is the one the subscription is connected to, or, before any video exists to connect it from, the one named by the rule's save folder (`<work>/Season NN` under the collect folder, read the way the renamer reads it). The earlier seasons' episodes are the sum of the AniList counts linked to the work's earlier seasons (season info); a missing season folder, link or count makes the sum unknown and nothing is guessed. When the first release is that sum plus one, the offset is set to minus the sum (`- 25` after 24 episodes gives `-24`, so the first video is `S03E01`) and the rule's detail shows `자동` with the grounds; `- 01` of a new work gives 0. In every other case the rule is received without conversion and the detail suggests a value with its grounds and `적용`, or says why it cannot suggest one. Only an offset of 0 or 1 that the app did not set is ever replaced, and only before the rule has picked an item: an item already received is never renamed, and a value changed by hand loses `자동` and is never set again. The app cannot tell an untouched 0 or 1 from one typed before the first pick, so a typed 0 or 1 is overwritten if the first release is exactly the sum plus one. A split cour that goes on numbering in the same season folder is not converted (a folder that already holds videos only gets a note). Nothing here changes the subtitle episode mapping.

### Watch folders and inotify

`trss-worker` watches the watch folders (including the collect and archive folders) with inotify. A new episode shows up in the library a few seconds after its file appears, without waiting for the 5-minute cycle, and only the work that changed is read again. Changes made by other containers on the same host (Transmission) are seen the same way. The cycle still reads a folder's directories by their modification times when the worker starts, when the kernel reports that it dropped events, for directories that could not be watched, and once an hour as a safety net (for example for a share that other machines change over SMB or NFS, which the kernel does not report).

Each watched directory (a watch folder, each work folder, each `Season NN` folder) takes one watch of the kernel's `fs.inotify.max_user_watches`, about 1 KB of kernel memory, counted per user for every process of that user on the host. A library of about 1,500 folders uses about 1,500 watches; the worker also opens one inotify instance per watch folder (`fs.inotify.max_user_instances`, 128 by default). When the limit is reached the watch folder's row in the settings says how many directories are not watched and why; the worker then checks those works itself every cycle, and nothing else changes. To watch them as well, raise the limit on the host, for example `echo 'fs.inotify.max_user_watches=524288' | sudo tee /etc/sysctl.d/60-inotify.conf` and `sudo sysctl --system`. Read the current value with `cat /proc/sys/fs/inotify/max_user_watches`, and see how many watches the worker holds with the `directories watched` line it logs when it starts.

### Channels and the collect folder

Channels and rules live in the app database and are edited in the web. To bring over a channel configuration of the old binary (the YAML at `CHANNELS_CONFIG_URL`), use Settings → Data → Import in the web. Importing only writes channels and rules (and the collect folder, below) and the subscriptions you check; it adds, renames and removes nothing.

The review step also reads the two comment lines directly above each rule (no blank line between) and suggests a subscription for that rule:

```yaml
# Mon. 23:30. <creator name>
# https://anissia.net/anime?animeNo=<number>
```

The first line is the weekday (`Mon` `Tue` `Wed` `Thu` `Fri` `Sat` `Sun`), the time (`HH:MM`) and the subtitle creator, separated by `. `; the creator is everything after the second separator. The second line is the Anissia address. A suggestion with a creator starts checked. A line 1 that does not fit (or has no creator) leaves an address-only suggestion, unchecked and shown as `제작자 미정`, and a comment with no readable Anissia address shows why it could not be read and imports the rule alone. Checked suggestions become subscriptions in the same import, with the weekday and time Anissia gives (the comment's own weekday and time are shown only while Anissia cannot be reached; the daily refresh fills in the stored ones). The import receives nothing, and the items already in the history count as past ones. A channel the import creates has no history yet, so a subscription leaves everything its feed holds at the channel's first read as past ones too (shown as `first_read` in the rule's 지난 회차, for the user to pick); plain rules receive the matching items of that first read as ever. The grammar is in `src/import/comments.rs`; adjust it there if your comments are written differently.

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

### Archive suggestions

The 구독 tab (beside the title candidates) and a rule's detail suggest archiving a rule that is not archived and has a match phrase (a subscription still waiting for its title has none), paused rules included. Nothing is switched off by itself, and the menu badge does not count suggestions. A suggestion is computed when read (`GET /api/archive-suggestions`, which the future 할 일 list reads too) from the stored Anissia snapshots and the history; nothing is asked of Anissia or the feed, so while Anissia cannot be reached the refresh finds nothing out and no `방영 종료` ground appears. Grounds:

- `방영 종료`: the subscription's anime has an end date that has passed (the weekly schedule drops its card by the same rule), or it has none and the daily refresh found it no longer listed.
- `새 항목 없음`: 4 weeks (exactly) have passed with no new item matching the rule. A *new item* is one the channel first recorded in the last 4 weeks whose title the rule matches (the channel's excludes and the rules ahead of it set aside as the rule preview does), whatever became of it; so what an earlier rule took, or a paused rule saw, counts. The 4 weeks count from the rule's last receive, and for a rule that never received from when it started collecting: the latest of when the app first had the rule, when it became a subscription or was given its title, when it was last turned back on or restored, and, for a rule from before the migration, when its channel was first read. A channel that recorded more than 20,000 items in 4 weeks gives its rules no `새 항목 없음` ground (the web logs it), and so does a rule whose regular expression does not compile.

`수집 유지` remembers the grounds it saw and nothing else changes about the rule: the same ground does not suggest it again (the end of the anime is told by its end date, the quiet stretch by the moment it counts from), and a new ground does (an ending, or a quiet stretch after another receive). Archiving from the suggestions sends each rule's own `rule_archive` command in the list's order, one after another, so a work folder shared by several of them moves once, with the last. The migration adds `rule_started` (stamped by triggers when a rule is created; rules from before it have no stamp) and `archive_suggestion_kept`.

### Video revisions

When a feed brings a higher revision of a release whose episode is already in the rule's folder (`[SubsPlease] Show - 14v2 (1080p) [1A2B3C4D].mkv` after `… - 14 …`), `trss-worker` receives it under its own name, checks that its CRC32 is the one in the name, and only then removes the old video and gives the new one the episode name. The work's episode row then shows a quiet `v1 › v2` line, so a subtitle timed for the old video can be checked.

- **The old video is deleted, for good.** If it is the only file of a torrent trss added, that torrent is removed from Transmission together with its data; if no torrent holds it, the file is deleted. There is no copy to restore. A video in a torrent of several files (a batch), in a torrent trss did not add, or named by two torrents at once is never touched: the replacement fails and both files stay. Which torrent holds a file is read from Transmission's file lists by path and confirmed on the file itself (same device and inode), so a folder Transmission names through a symbolic link still matches; Transmission and the worker must still see the media at the same path (`/downloads`), as for the season link.
- **What is deleted is checked again just before.** The worker looks at the episode's file again right before removing it: a torrent's file must be a lower revision of the same release, and a file of no torrent must still have the CRC32 it had when the revision was decided (it is read again for that). Then, immediately before the torrent is removed or the file deleted, the path must still be the file that was checked (same device, inode, size, modification time and status-change time, so a rewrite that puts the modification time back is caught too); a file replaced or changed in between, or while its CRC32 was read, is not deleted, and a file that is gone by then counts as removed. Anything else fails the replacement and keeps the file. A video inside a torrent's own folder is never replaced, since removing that torrent would take the folder.
- **One revision at a time, never downwards.** When `14v2` and `14v3` are both on their way, only one replacement removes the episode's video at a time, the lower one is skipped once the higher one is in place or on its way (its video stays under its received name), and a lower revision that appears later is recorded as a duplicate and not received. If the higher one then fails (`받기 실패`), the lower one skipped for it goes back to its first step on a later cycle and replaces the old video after all, with the same checks; the higher one stays a failure with `다시 받기`, and once received it replaces the lower one the same way. The same release reaching the folder through two channels is one torrent and one replacement, and once its old video is replaced, neither channel's old item is received again.
- **A failure before the old video is removed keeps it** (the CRC32 differs, the old torrent could not be removed, the file could not be deleted). The episode row shows `받기 실패` with both files and why, and so does `GET /api/todo/receive-failures`; it goes away once one of the two files is deleted. A failure before the new video was received (its torrent stopped, reports a local error, or sits outside the rule's folder) is looked at again every cycle: once the torrent is right the replacement goes on, and a cycle that still sees the item in its feed adds it again when the torrent is gone. After the old video is removed, the worker renames the new one every cycle until the episode name is free and the rename goes through; it never renames over a file. A torrent removal Transmission did not answer is looked at again (it may have happened), and an old torrent removed while its file stayed waits, shown as `받기 실패`, until that file is gone. If the new video is deleted instead, or goes missing after the old video was removed and before it took the episode name, the replacement ends once two looks in a row (one per cycle) find it gone (one look alone may be a mount that is away for a moment): nothing is deleted or renamed, the old release is not received again, and later revisions of the episode no longer wait for it, with no need to put the file back.
- **`다시 받기` for a download that stopped.** When the revision's torrent left Transmission or reported an error before the video was received, the episode row (and the failure source) offers `다시 받기`, also after the release left the feed or when it came from a past episode search. It adds the torrent again from the link recorded in history, never one sent by the browser (or starts the one Transmission still has) and the replacement starts over with the same checks; the old video stays until the new one is checked. It is not offered while the rule is paused or archived, once a higher revision of the episode is in place or on its way, or when the rule's folder is no longer the one the replacement was decided for (a torrent added now would land away from the old video).
- **Nothing ever takes a name that is taken.** Transmission's rename leaves the file where it is and still answers `success` when the new name exists, so every rename (the episode name after an add, `다시 받기`, a replacement) is now skipped when that name is already in the folder; the video then keeps the name it was received under. This applies to the old binary too.
- **A revision is named as its episode.** Every rename derives the episode name from the release name without its revision marker (`06v2` → `06`): `trname` reads Erai-raws' `[Erai-raws] Show - 06v2 [1080p CR WEBRip HEVC AAC][MultiSub][1BBD34E6].mkv` as another episode, taken from the CRC32 bracket. The revision marker is the last `NvM` before the CRC32 bracket, so a show named `Show 3v3` keeps its `3v3`; a `NvM` followed by a bare number outside brackets is part of the name too, so `Show 3v3 - 06` is the first revision of episode 6. History keeps the release's own title.
- A revision whose name has no CRC32, or whose existing video matches none of the release's revisions, is not received on its own: it shows as `버전 미상` in the collect history, and `다시 받기` receives it and replaces the old video (without the CRC32 check when the name has none). Another group's release of the same episode is no revision and is received as before, under its own name.
- **CRC32 cost.** The received file is read once, in 1 MiB blocks, when its download has finished: memory stays at that block whatever the file's size, but the worker reads the whole file from the disk (about 1.5 GB for a 24-minute 1080p episode), which takes seconds to tens of seconds on a hard disk and delays the rest of that cycle. A video already in the folder whose torrent history does not know is read the same way when its revision first appears, and once more right before it is deleted. Only one file is read at a time. The outcome is stored, so a decision is not read again.
- **Transmission's whole file list** (every torrent with its files) is asked for at most once per cycle for the decisions, and only when a revision's episode file is not its own torrent's; the replacement steps ask for it again only after they changed Transmission.
- Each replacement's progress is stored (migration 25, the `video_revisions` table), and every step is written before it acts, so a worker stopped halfway carries on from what is on disk when it starts again; a step that acted but was not written (the rename went through, say) is recognized from the disk on the next look. A revision's decision is written in one transaction with its item's history, so an item is never recorded as received or `버전 미상` without it.

### Past episode search

A rule's detail has a `지난 회차 검색` section for episodes the feed no longer holds: confirm the query and the release range, preview the judged results, then receive the ones you tick. The search is one read of the tracker's search RSS (never the HTML search page) made by `trss-web` for that one search; it creates no channel, and a search you leave or finish keeps nothing, so the history holds only what was received. A search is kept in the web process's memory for 30 minutes (at most 6 at once; a new search of a rule ends its old one), so restarting `trss-web` ends running searches. The picked results are sent by key and `trss-worker` adds them; the browser never sends a link.

- **Query.** The channel's search format (`[SubsPlease] {match} 1080p`, set in the channel tab) with the rule's match phrase put where `{match}` is. A channel without a format starts from the match phrase, and you edit the query in place. A regular-expression rule and a rule that is still waiting for its title have no phrase to search for, so they start with an empty box and a sentence saying why. The URL is the channel's own with `q` set to the query (and `p` dropped); the channel's other query values (category, filter, token) are kept.
- **Pace.** Requests to one host are at least 3 seconds apart, however many searches or `trss-web` processes ask: the next allowed time of each host is a row of `search_pace` (migration 26), and each request takes its slot in one write transaction before it is sent. Only search requests take slots, and only the web sends them: the worker does not use the row, and its cycle reads of the feed are unchanged. A request waits for its slot at most 60 seconds; when the slot is further away (the host is blocked, or the clock moved), the search fails at once and says when to try again. A `429` ends the search with the tracker's `Retry-After` and blocks that host's slots until then, and a request that was waiting for its slot is not sent into the block.
- **Long series.** The tracker returns at most 75 results. When the first page is full, the episodes of the range that are still missing are searched in groups of 10, `{series} - (1000|1001|…)`, using the numbering of the first page's real titles, and merged without repeats. At most 20 extra searches are sent (so a search takes up to a minute), a range spans at most 2000 episodes, a search keeps at most 2000 results, and at most 20 existing videos are read for their CRC32.
- **What is selected.** Only an episode that is missing from the work folder and that no history of the rule or channel shows as received. A higher revision of an existing episode and a result whose version cannot be told (`버전 미상`) are shown but not selected; tick one to receive it with the checks of video revisions above. Batches, episodes already in the folder and episodes of another season are never selected, and results outside the range are folded into a count. Transmission is not asked: whether an episode was received from another release is judged from the recorded history and the real files in the work folder.

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
