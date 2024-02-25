FROM rust:1.76-bookworm AS builder
RUN apt-get update
RUN apt-get install -y protobuf-compiler pkgconf libssl-dev
WORKDIR /usr/src/pipo
COPY . .

FROM builder AS debug
RUN --mount=type=cache,target=$HOME/.cargo/bin,sharing=locked \
    --mount=type=cache,target=$HOME/.cargo/registry/index,sharing=locked \
    --mount=type=cache,target=$HOME/.cargo/registry/cache,sharing=locked \
    --mount=type=cache,target=$HOME/.cargo/git/db,sharing=locked \
    --mount=type=cache,target=/usr/src/pipo/target,sharing=locked \
    cargo install --debug --path .
    
FROM builder AS release
RUN --mount=type=cache,target=$HOME/.cargo/bin,sharing=locked \
    --mount=type=cache,target=$HOME/.cargo/registry/index,sharing=locked \
    --mount=type=cache,target=$HOME/.cargo/registry/cache,sharing=locked \
    --mount=type=cache,target=$HOME/.cargo/git/db,sharing=locked \
    --mount=type=cache,target=/usr/src/pipo/target,sharing=locked \
    cargo install --path .
CMD ["pipo"]

FROM rust:1.76-slim-bookworm
RUN apt-get update \
    && apt-get install -y libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=release /usr/local/cargo/bin/pipo /usr/local/bin/pipo
CMD ["pipo"]
