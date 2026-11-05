# B-route clipboard e2e: a container that runs the real `mixumd` (agentd) plus
# the Smithay compositor, driven from the host over MAP.
#
# Both binaries are copied into the build context by tests/e2e/b-clipboard.mjs:
#   mixumd                  static musl build of mixum-agentd (mixumOS)
#   mixum-env-compositor    release build of crates/mixum-env-compositor
#
# Trixie (glibc 2.41) because the compositor is built on rust:latest.
FROM debian:trixie-slim

RUN set -eux; \
    if [ -f /etc/apt/sources.list.d/debian.sources ]; then \
      sed -i 's#deb.debian.org#mirrors.aliyun.com#g' /etc/apt/sources.list.d/debian.sources; \
    else \
      sed -i 's#deb.debian.org#mirrors.aliyun.com#g' /etc/apt/sources.list; \
    fi; \
    apt-get update -qq; \
    apt-get install -y -qq --no-install-recommends \
        libxkbcommon0 \
        libegl1 \
        libgl1-mesa-dri \
        foot \
        wl-clipboard \
        xclip \
        x11-utils \
        xwayland \
        procps \
        ca-certificates; \
    rm -rf /var/lib/apt/lists/*

COPY mixumd /usr/local/bin/mixumd
COPY mixum-env-compositor /usr/local/bin/mixum-env-compositor
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /usr/local/bin/mixumd /usr/local/bin/mixum-env-compositor /entrypoint.sh

EXPOSE 7601
ENTRYPOINT ["/entrypoint.sh"]
