# SIIISHUB in the browser

`siiishub-server` serves the interface of the app to a browser, from a server
or a Docker container: catalog, details, streams from the addons, playback,
library, settings and downloads work as in the app, with the torrents and
the downloads on the server. Every browser gets the interface of the
Windows and Linux app, on phones and tablets too, where it adapts to the
narrower screen.

## Playback

The browser plays what it can as it is; the rest comes as HLS, which the
server makes with FFmpeg while the film plays, as Stremio's streaming server
does:

- **As it is**, with native seeking: MP4, or Matroska on Chromium browsers,
  with a video and an audio the browser decodes and a single audio track.
- **HLS, video copied** when the browser decodes it (H.264, and HEVC or AV1
  where the browser does, 10-bit HDR included), the audio in stereo AAC.
  Light on the server: this is how most files play, EAC3, DTS and files with
  several audio tracks included.
- **HLS, video transcoded** to 1080p H.264 when the browser cannot decode
  it, with HDR mapped to SDR: on a GPU when the server has one (below),
  otherwise on the CPU, where it is heavy: a 4K HDR10 HEVC film needs about
  a quarter of a 16-thread CPU (4.3x real time in a test).

The HLS playlist covers the whole film in 6-second segments, made on demand
from the one the player asks for. Seeking is the player's own: into what was
already made it takes a fraction of a second, elsewhere FFmpeg starts over
there (2-3 seconds from a debrid link). Up to 3 minutes are made ahead of
the player and 12 minutes are kept on disk (in the system's temporary
folder), the farthest from the player going first. hls.js plays it, loaded
from jsdelivr; Safari plays HLS by itself; without either, a single stream
restarted at every seek takes over.

A change of audio track opens a new HLS session from the same second (about
2 seconds). Speed, volume up to 200% and the resume position work as in the
app, and the preferred audio and subtitle languages of the settings pick the
tracks.

Subtitles: the ones of the subtitle addons (converted to WebVTT, SRT in
UTF-8 or Windows-1252, ASS through FFmpeg), and the text ones inside the file
(SRT, ASS, WebVTT): FFmpeg extracts all of them while it makes the segments,
so switching between them is immediate. Bitmap subtitles (PGS, DVD) are not
shown.

Dolby Vision without an HDR10 base (profile 5) only Safari shows right:
elsewhere its colours come out green and purple, and the player says so
over the video.

## Phone remote

As in the app: Settings → Remote shows the address and the QR code of the
phone page, `/remote/` on this server. The phone signs in with the same
password, then the screen asks to approve it (and can remember it, so it is
not asked again). The phone drives playback (pause, seek, volume, tracks),
moves around the interface with its arrows and searches; every open page of
the interface gets its commands. Full screen cannot be switched from the
phone: browsers allow it only after a click on the page.

## Docker

```sh
docker compose up -d
```

The image is `ghcr.io/spettrix01/siiishub`: `latest` follows the main
branch, and each release has its version (`1.2.0`), built by
`.github/workflows/docker.yml`. To build it from the sources instead, replace
`image:` with `build: .` in `docker-compose.yml` (a Rust build: some minutes
and a few GB of memory).

Then open `http://<server>:8080` and sign in with the password set in
`docker-compose.yml` (`SIIISHUB_PASSWORD`). The settings, the library and the
sessions are kept in `./config`, the downloads in `./downloads`.

The container runs as user 1000: the two folders must be writable by it
(`sudo chown -R 1000:1000 config downloads` if they were created by root).

### TrueNAS SCALE

Create a dataset for it (here `pool/Siiishub`) and make the `apps` user
(568) its owner: Datasets → the dataset → Permissions → Edit, user and
group `apps`. Then Apps → Discover Apps → ⋮ → Install via YAML, a name
(`siiishub`) and this configuration, with a password of your own:

```yaml
services:
  siiishub:
    image: ghcr.io/spettrix01/siiishub:latest
    restart: unless-stopped
    user: "568:568"
    # The graphics for transcoding (Intel, AMD): TrueNAS's video (44) and
    # render (107) groups.
    group_add: ["44", "107"]
    devices:
      - /dev/dri:/dev/dri
    ports:
      - "30808:8080"
    environment:
      SIIISHUB_PASSWORD: "change-me"
      SIIISHUB_DATA_DIR: /data/config
      SIIISHUB_DOWNLOAD_DIR: /data/downloads
    volumes:
      - /mnt/pool/Siiishub:/data
```

The server makes `config` and `downloads` in the dataset, and answers on
`http://<truenas>:30808`. The app's page in TrueNAS shows its log:
`[transcode]` says whether it found the graphics.

## Without Docker

```sh
cd src-tauri
cargo build --release --no-default-features --features server --bin siiishub-server
cd ..
SIIISHUB_PASSWORD=... ./src-tauri/target/release/siiishub-server
```

Run it from the repository root, or point `SIIISHUB_APP_DIR` and
`SIIISHUB_WEB_DIR` at `dist/` and `web/`. It needs `ffmpeg` and `ffprobe` on
the `PATH`, with libx264 and zscale (Debian's and Ubuntu's FFmpeg have both),
and to transcode on a GPU its driver: `intel-media-va-driver-non-free` or
`mesa-va-drivers` for Intel and AMD graphics, NVIDIA's own for NVIDIA.

## Transcoding on a GPU

Most videos play without being transcoded (as they are, or with the video
copied). The others (HEVC where the browser lacks it, older codecs such as
MPEG-2, VC-1 or XviD, 10-bit H.264) are, and a GPU does it for a fraction of
what the CPU spends. At startup the server tries the GPUs it sees, as
Stremio's streaming server does, and keeps the first that works; the log
says which, or why none (`[transcode] on the GPU: ...`, `[transcode] on the
CPU: ...`).

- **Intel and AMD graphics** (VAAPI), the integrated ones of mini PCs and
  NAS included: give the container `/dev/dri` and the host's render group
  (`devices` and `group_add` in `docker-compose.yml`; the group's id is
  `getent group render | cut -d: -f3` on the host). Every render node is
  tried, so on a computer with two GPUs the one that encodes is found even
  when it is not `renderD128`. Intel's driver maps HDR10 to SDR on the GPU
  too; with AMD's, that step alone runs on the CPU.
- **NVIDIA** (NVENC): install the NVIDIA Container Toolkit on the host and
  uncomment `deploy` in `docker-compose.yml`. HDR is mapped to SDR on the
  CPU, the rest runs on the GPU: in a test, a 4K HDR10 film took half the
  CPU time of a transcode on the CPU alone, an SDR film next to none.

The GPU decodes the codecs it knows; FFmpeg decodes the others and hands
the frames over. If the GPU still fails a transcode (all its encoder
sessions busy, a driver that does not cope), the transcode starts again on
the CPU by itself.

## Configuration

| Variable | Default | |
|---|---|---|
| `SIIISHUB_PASSWORD` | none, required | Password to sign in with. |
| `SIIISHUB_AUTH` | | `off` runs without a login: only on a network you trust. |
| `SIIISHUB_PORT` | `8080` | |
| `SIIISHUB_ADDRESS` | `0.0.0.0` | Address to listen on (`127.0.0.1` behind a reverse proxy on the same machine). |
| `SIIISHUB_DATA_DIR` | `data` (`/config` in Docker) | Settings, library, sessions, torrent state. |
| `SIIISHUB_DOWNLOAD_DIR` | `<data>/download` (`/downloads` in Docker) | |
| `SIIISHUB_APP_DIR` | `dist` (`/app/dist` in Docker) | The app's interface. |
| `SIIISHUB_WEB_DIR` | `web` (`/app/web` in Docker) | The browser additions. |
| `SIIISHUB_HWACCEL` | `auto` | The GPU to transcode on: `auto` (the first that works), `vaapi`, `nvenc`, or `off` for the CPU. |
| `SIIISHUB_HWACCEL_DEVICE` | | Its render node (`/dev/dri/renderD129`) or NVIDIA GPU number (`1`), instead of the first that works. |
| `RUST_LOG` | `info,siiishub_desktop_lib=debug` | Log levels. |

## Security

- Every page and every API call needs a session, except the login page and
  what it loads (the styles and the translations). A session lasts 30 days
  and survives restarts (`web-sessions.json`, readable only by the server's
  user). Ten wrong passwords in a minute block the login until the minute is
  over.
- On the internet, put the server behind a reverse proxy with HTTPS (Caddy,
  Traefik, Nginx Proxy Manager). When the proxy sends
  `X-Forwarded-Proto: https`, the session cookie is only sent encrypted.
- The API refuses requests whose `Origin` is not the server itself, so pages
  of other sites cannot use the session. The proxy must pass the original
  `Host` header (or `X-Forwarded-Host`); Caddy and Traefik do by default,
  Nginx needs `proxy_set_header Host $host;`.
- Without a debrid service, torrents are downloaded from the server's
  internet connection.

## Architecture

- `src-tauri/src/ops/`: what the interface asks of the backend (settings,
  library, addons, resolving streams, torrents, downloads). The app's Tauri
  commands (`commands/`) and the server's API are thin layers over it.
- `src-tauri/src/server/`: the HTTP server (axum). `POST /api/invoke/<command>`
  runs a command with the arguments the page passes to `invoke`; the events
  (`media://progress`, ...) go to the pages over a WebSocket (`/api/events`).
  The same crate builds the app (feature `app`, default) or the server
  (feature `server`), which leaves out Tauri, libmpv and GTK.
- `src-tauri/src/remote.rs`: the phone remote (the phone page, approvals,
  pairings), the app's and the server's: the app serves it on a port of its
  own, the server at `/remote/` (`server/remote.rs`) with its events going to
  the pages over `/api/events`.
- `web/bridge.js` stands in for `window.__TAURI__` in the browser; the server
  injects it into the app's `index.html`, with `web/web.css` and `web/web.js`
  (the theme for the login page). The interface itself is the app's:
  `IS_WEB` (`js/platform.js`) and the `is-web` class hide what has no place
  in a browser (window controls, remote control, opening folders); the
  Android flags stay false in a browser on Android, whose layout is the
  app's. `index.html` loads the interface from `/v/<version>/` (the time of
  its newest file): browsers keep those files for good and fetch them again
  after an update, when the version changes. The rest is sent `no-cache`.
- `web/player.js` takes the `mpv_*` commands of the app's player interface
  and answers with the same properties and events as mpv, over an HTML5
  video: the interface does not know the difference.
- `src-tauri/src/server/media.rs`: the streams the backend resolves (torrent
  session URLs on 127.0.0.1, debrid links, files on disk) only the server
  reaches, so the page gets `/media/<id>` instead, served as it is with byte
  ranges (or, as the fallback, through FFmpeg as one fragmented MP4).
- `src-tauri/src/server/hls.rs`: the HLS sessions. One FFmpeg job at a time
  per session writes fragmented MP4 to its stdout, which is cut into the
  playlist's segments. FFmpeg starts every output at time zero, so the
  fragments' decode times (`tfdt`) are moved to the film's time where the job
  started: the target when transcoding, the keyframe before it when copying,
  as a one-packet `framecrc` side output of the same job reports it.
  Transcoded keyframes are forced on the 6-second grid of the film. The init
  segment comes from the first job that makes a fragment, so a GPU that
  fails before that leaves nothing of its own in the session.
- `src-tauri/src/server/transcode.rs`: the GPU test at startup (a second of
  1440p SDR and one of 720p HDR10 through the pipeline of a real transcode,
  with a time limit: a driver can hang), the filter chains and encoders for
  VAAPI, NVENC and the CPU, and the steps back after a failure (HDR mapped
  on the CPU, then the CPU alone).
