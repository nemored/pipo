FROM rust:1.65-alpine

RUN apk add --update openssl

WORKDIR /usr/src/pipo
COPY . .

RUN cargo install --path .

CMD ["pipo"]
