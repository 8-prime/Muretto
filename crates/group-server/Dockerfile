# The group server, as one static binary in an otherwise empty image. Build
# from the repository root:
#
#   docker build -f crates/group-server/Dockerfile -t muretto-group-server .
#   docker run --rm -p 3100:3100 muretto-group-server
#
# Put it behind a TLS proxy that forwards WebSocket upgrades on /v1/, and
# point Muretto at the wss:// address. The server makes no connections of its
# own, so the image needs no certificates.

# On Alpine, Rust targets musl and links statically by default.
FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p muretto-group-server

FROM scratch
COPY --from=build /src/target/release/muretto-group-server /muretto-group-server
# No /etc/passwd in scratch: run as a numeric, unprivileged user.
USER 10001:10001
ENV GROUP_ADDR=0.0.0.0:3100
EXPOSE 3100
ENTRYPOINT ["/muretto-group-server"]
