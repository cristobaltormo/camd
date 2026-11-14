FROM rust:1-alpine AS build
RUN apk add --no-cache gcc musl-dev
WORKDIR /src
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY web ./web
ENV CFLAGS=-DSQLITE_DISABLE_LFS
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release --locked \
 && cp target/release/camd /camd

FROM alpine:3.22 AS runtime
RUN apk add --no-cache ffmpeg iputils-ping iputils-arping ca-certificates tzdata libcap-setcap \
 && setcap cap_net_raw+ep "$(command -v arping)" \
 && setcap cap_net_raw+ep "$(command -v ping)" \
 && apk del --no-cache libcap-setcap \
 && delgroup ping \
 && addgroup -S -g 999 camd \
 && adduser -S -D -H -u 999 -G camd -h /data -s /sbin/nologin camd \
 && mkdir /data \
 && chown camd:camd /data
ENV CAMD_LISTEN_HOST=0.0.0.0 \
    CAMD_DATA_DIR=/data
VOLUME /data
EXPOSE 8180
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s \
    CMD wget -q -T 3 -O /dev/null "http://127.0.0.1:${CAMD_LISTEN_PORT:-8180}/healthz" || exit 1
USER camd
ENTRYPOINT ["camd"]

FROM runtime AS prebuilt
ARG TARGETARCH
COPY --chmod=0755 dist/camd-linux-${TARGETARCH} /usr/local/bin/camd

FROM runtime
COPY --from=build /camd /usr/local/bin/camd
