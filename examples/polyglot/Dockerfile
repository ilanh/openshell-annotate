# @openshell egress deb.debian.org:443 reason="base image packages" access=read-only bin=/usr/bin/apt-get
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates
