FROM rust:1.73-buster as builder
RUN apt-get update
RUN apt-get install -y protobuf-compiler pkgconf libssl-dev
WORKDIR /usr/src/pipo
COPY . .
RUN --mount=type=cache,target=$HOME/.cargo/bin,sharing=locked \
    --mount=type=cache,target=$HOME/.cargo/registry/index,sharing=locked \
    --mount=type=cache,target=$HOME/.cargo/registry/cache,sharing=locked \
    --mount=type=cache,target=$HOME/.cargo/git/db,sharing=locked \
    --mount=type=cache,target=/usr/src/pipo/target,sharing=locked \
    cargo install --path .

FROM rust:1.73-slim-buster
RUN apt-get update \
    && apt-get install -y libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/pipo /usr/local/bin/pipo
CMD ["pipo"]
