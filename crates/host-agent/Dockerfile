# AssettoServer with Muretto's host agent in charge of it. Build from the
# repository root:
#
#   docker build -f crates/host-agent/Dockerfile -t muretto-host .
#   docker run -d --init --name muretto-host \
#     -e AGENT_TOKENS='stefan:<at least 16 characters>' \
#     -e SERVER_NAME="Stefan's server" \
#     -v muretto-host:/data \
#     -p 3200:3200 -p 9600:9600 -p 9600:9600/udp -p 8081:8081 \
#     muretto-host
#
# The agent replaces AssettoServer's entrypoint and runs the server as its
# child. Everything lives in /data: the server's cfg/ and content/ as usual,
# and the agent's presets in /data/muretto. Put the agent's port behind a TLS
# proxy; the tokens travel with every request. `--init` reaps any processes
# the server leaves behind.

# Pinned: the agent writes config for this version. Stable releases are
# rare; the pre-releases are what AssettoServer publishes.
ARG ASSETTOSERVER_VERSION=0.0.55-pre49

# On Alpine, Rust targets musl and links statically by default, so the agent
# runs on the server image's Debian without anything of its own.
FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p muretto-host-agent

FROM compujuckel/assettoserver:${ASSETTOSERVER_VERSION}
COPY --from=build /src/target/release/muretto-host-agent /app/muretto-host-agent
ENV AGENT_ADDR=0.0.0.0:3200 \
    SERVER_DIR=/data \
    SERVER_COMMAND=/app/AssettoServer \
    SERVER_ARGS=--plugins-from-workdir
VOLUME /data
EXPOSE 3200 9600 9600/udp 8081
ENTRYPOINT ["/app/muretto-host-agent"]
