FROM rust:1-bookworm AS builder

WORKDIR /app/server
COPY server/Cargo.toml server/Cargo.lock ./
COPY server/src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim

ARG DENO_VERSION=2.5.6

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates python3 python3-pip curl unzip \
    && python3 -m pip install --break-system-packages --no-cache-dir yt-dlp yt-dlp-ejs \
    && curl -fsSL "https://github.com/denoland/deno/releases/download/v${DENO_VERSION}/deno-x86_64-unknown-linux-gnu.zip" -o /tmp/deno.zip \
    && unzip /tmp/deno.zip -d /usr/local/bin \
    && chmod +x /usr/local/bin/deno \
    && rm -f /tmp/deno.zip \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/server/target/release/bkgrnd_server /usr/local/bin/bkgrnd_server
COPY server/web /app/web

ENV WOPR_BIND=0.0.0.0:808
ENV WOPR_WEB_DIR=/app/web
ENV WOPR_DATA_DIR=/data
ENV WOPR_YTDLP_JS_RUNTIMES=deno:/usr/local/bin/deno
ENV RUST_LOG=info

EXPOSE 808
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=5 \
    CMD python3 -c "import urllib.request; urllib.request.urlopen('http://127.0.0.1:808/api/v1/health', timeout=3).read()" || exit 1

CMD ["bkgrnd_server"]
