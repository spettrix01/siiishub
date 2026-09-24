# SIIISHUB in the browser (siiishub-server), published as
# ghcr.io/spettrix01/siiishub (.github/workflows/docker.yml). Built by hand
# from the repository root:
#   docker build -t siiishub .
# docs/WEB.md explains the configuration; docker-compose.yml is an example.

FROM rust:1-bookworm AS build
WORKDIR /src
COPY src-tauri ./src-tauri
RUN cd src-tauri \
 && cargo build --release --locked --no-default-features --features server --bin siiishub-server

# Debian 13: FFmpeg 7 for the video pipeline of the browser player, and the
# VAAPI drivers to transcode on Intel and AMD graphics (docs/WEB.md): Mesa's
# for AMD, Intel's full ones from non-free for Intel (x86 only; i965 for the
# graphics older than 2015).
FROM debian:trixie-slim
RUN sed -i 's/^Components: main$/Components: main non-free/' /etc/apt/sources.list.d/debian.sources \
 && apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates ffmpeg mesa-va-drivers \
      $(if [ "$(dpkg --print-architecture)" = amd64 ]; then echo intel-media-va-driver-non-free i965-va-driver-shaders; fi) \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --uid 1000 --no-create-home --home-dir /config --shell /usr/sbin/nologin siiishub \
 && mkdir -p /config /downloads \
 && chown siiishub:siiishub /config /downloads
WORKDIR /app
COPY --from=build /src/src-tauri/target/release/siiishub-server /app/siiishub-server
COPY dist /app/dist
COPY web /app/web
# With an NVIDIA GPU, its container toolkit also brings the encoder and the
# decoder of the driver (video) and nvidia-smi (utility).
ENV SIIISHUB_DATA_DIR=/config \
    SIIISHUB_DOWNLOAD_DIR=/downloads \
    SIIISHUB_APP_DIR=/app/dist \
    SIIISHUB_WEB_DIR=/app/web \
    SIIISHUB_PORT=8080 \
    NVIDIA_DRIVER_CAPABILITIES=compute,video,utility
VOLUME ["/config", "/downloads"]
EXPOSE 8080
USER siiishub
ENTRYPOINT ["/app/siiishub-server"]
