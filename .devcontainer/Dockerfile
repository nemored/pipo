FROM debian:13.4

RUN DEBIAN_FRONTEND=noninteractive apt-get -y update && apt-get -y install --no-install-recommends bubblewrap

RUN groupadd --gid 1000 agent && \
    useradd --uid 1000 --gid agent --shell /bin/bash --create-home codex

USER codex
WORKDIR /home/codex
