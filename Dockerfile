FROM rust:1.67-alpine as builder
RUN apk add libc-dev openssl-dev pkgconfig protoc
WORKDIR /usr/src/pipo
COPY . .
RUN cargo install --path .

FROM rust:1.67-alpine
COPY --from=builder /usr/local/cargo/bin/pipo /usr/local/bin/pipo
CMD ["pipo"]
