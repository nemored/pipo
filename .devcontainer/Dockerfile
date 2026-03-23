FROM debian:13.4

RUN groupadd --gid 1000 agent && \
    useradd --uid 1000 --gid agent --shell /bin/bash --create-home codex

USER codex
WORKDIR /home/codex
