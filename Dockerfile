FROM rust:1.67-alpine as builder
RUN apk add libc-dev openssl-dev pkgconfig protoc sqlite-dev
WORKDIR /usr/src/pipo
COPY . .
RUN --mount=type=cache,target=$HOME/.cargo/bin,sharing=locked \
    --mount=type=cache,target=$HOME/.cargo/registry/index,sharing=locked \
    --mount=type=cache,target=$HOME/.cargo/registry/cache,sharing=locked \
    --mount=type=cache,target=$HOME/.cargo/git/db,sharing=locked \
    --mount=type=cache,target=/usr/src/pipo/target,sharing=locked \
    cargo install --path .

FROM rust:1.67-alpine
COPY --from=builder /usr/local/cargo/bin/pipo /usr/local/bin/pipo
CMD ["pipo"]
