FROM rust:1-bookworm AS builder

WORKDIR /app/server
COPY server/Cargo.toml server/Cargo.lock ./
COPY server/src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim

ARG TARGETARCH
ARG DENO_VERSION=2.9.1
ARG DENO_AMD64_SHA256=710c54d63477d1100844ef4818f19507ce0dbf40510903b1d883f19e394446a2
ARG DENO_ARM64_SHA256=0a60d079fa79635a59803074dbbfe86ccc35746dc2c4f8d73f2e50338b3283a9
ARG BGUTIL_PROVIDER_COMMIT=fbe4ed47f3b63cf061f1158f18f74bcc90e54033
ARG BGUTIL_PROVIDER_ARCHIVE_SHA256=cbc8c2e54126ec38f4c2a278b3cab685d337cadc3e7f09762116e3b28be18b5f

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates python3 python3-pip curl unzip zip \
    && printf '%s\n' \
      'yt-dlp==2026.8.19 --hash=sha256:1d57897e94c6665a0a6f9bc54b34e584284e32c034ffab3a7df25d8f7b24eedf' \
      'yt-dlp-ejs==0.8.0 --hash=sha256:79300e5fca7f937a1eeede11f0456862c1b41107ce1d726871e0207424f4bdb4' \
      > /tmp/yt-dlp-requirements.txt \
    && python3 -m pip install --break-system-packages --no-cache-dir --require-hashes -r /tmp/yt-dlp-requirements.txt \
    && case "${TARGETARCH}" in \
      amd64) deno_asset=deno-x86_64-unknown-linux-gnu.zip; deno_sha256="${DENO_AMD64_SHA256}" ;; \
      arm64) deno_asset=deno-aarch64-unknown-linux-gnu.zip; deno_sha256="${DENO_ARM64_SHA256}" ;; \
      *) echo "Unsupported TARGETARCH: ${TARGETARCH}" >&2; exit 1 ;; \
    esac \
    && curl -fsSL "https://github.com/denoland/deno/releases/download/v${DENO_VERSION}/${deno_asset}" -o /tmp/deno.zip \
    && echo "${deno_sha256}  /tmp/deno.zip" | sha256sum -c - \
    && unzip /tmp/deno.zip -d /usr/local/bin \
    && chmod +x /usr/local/bin/deno \
    && rm -f /tmp/deno.zip /tmp/yt-dlp-requirements.txt \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

# Patched PR #243 provider. Both the revision and downloaded bytes are pinned.
# The upstream 1.x listener is rewritten before installation because it has no
# authentication and must never be reachable outside this container.
RUN curl -fsSL \
      "https://codeload.github.com/Brainicism/bgutil-ytdlp-pot-provider/tar.gz/${BGUTIL_PROVIDER_COMMIT}" \
      -o /tmp/bgutil-provider.tar.gz \
    && echo "${BGUTIL_PROVIDER_ARCHIVE_SHA256}  /tmp/bgutil-provider.tar.gz" | sha256sum -c - \
    && mkdir -p /opt/bgutil-provider /opt/yt-dlp-plugins \
    && tar -xzf /tmp/bgutil-provider.tar.gz -C /opt/bgutil-provider --strip-components=1 \
    && sed -i \
      -e 's/host: "::"/host: "127.0.0.1"/' \
      -e 's/host: "0.0.0.0"/host: "127.0.0.1"/' \
      /opt/bgutil-provider/server/src/main.ts \
    && ! grep -Eq 'host: "(::|0\.0\.0\.0)"' /opt/bgutil-provider/server/src/main.ts \
    && cd /opt/bgutil-provider/server \
    && deno install --prod --allow-scripts=npm:canvas --frozen \
    && cd /opt/bgutil-provider/plugin \
    && zip -qr /opt/yt-dlp-plugins/bgutil-ytdlp-pot-provider.zip yt_dlp_plugins \
    && unzip -tq /opt/yt-dlp-plugins/bgutil-ytdlp-pot-provider.zip \
    && rm -f /tmp/bgutil-provider.tar.gz

WORKDIR /app
COPY --from=builder /app/server/target/release/bkgrnd_server /usr/local/bin/bkgrnd_server
COPY server/web /app/web
COPY server/container-entrypoint.sh /usr/local/bin/bkgrnd-container-entrypoint
RUN chmod 0755 /usr/local/bin/bkgrnd-container-entrypoint

ENV WOPR_BIND=0.0.0.0:808
ENV WOPR_WEB_DIR=/app/web
ENV WOPR_DATA_DIR=/data
ENV WOPR_YTDLP_JS_RUNTIMES=deno:/usr/local/bin/deno
ENV WOPR_YTDLP_PLUGIN_DIR=/opt/yt-dlp-plugins
ENV RUST_LOG=info

EXPOSE 808
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=5 \
    CMD python3 -c "import urllib.request; urllib.request.urlopen('http://127.0.0.1:808/api/v1/health', timeout=3).read()" || exit 1

ENTRYPOINT ["/usr/local/bin/bkgrnd-container-entrypoint"]
