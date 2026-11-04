# apt-get runs during `docker build`, not inside the sandbox, so it needs no egress rule.
# @openshell ignore kind=net reason="build-time only: image packages are installed before the sandbox starts"
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates
