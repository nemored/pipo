FROM ircd/unrealircd:nightly
USER root
RUN apk add --no-cache openssl \
    && openssl req \
    -x509 \
    -newkey rsa:4096 \
    -keyout /ircd/tls/server.key.pem \
    -out /ircd/tls/server.cert.pem \
    -sha256 \
    -days 30 \
    -nodes \
    -subj "/CN=ircd.local" \
    && chown -R unreal:unreal /ircd/tls
USER unreal
CMD ["/app/unrealircd/bin/unrealircd", "-F"]
