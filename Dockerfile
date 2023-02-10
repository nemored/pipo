FROM rust:1.67-alpine as builder
RUN apk add libc-dev openssl-dev pkgconfig protoc sqlite-dev
WORKDIR /usr/src/pipo
RUN --mount=type=cache,target=/usr/src/pipo/target,sharing=locked
COPY . .
RUN cargo install --path .

FROM rust:1.67-alpine
COPY --from=builder /usr/local/cargo/bin/pipo /usr/local/bin/pipo
CMD ["pipo"]
