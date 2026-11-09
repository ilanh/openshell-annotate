# syntax=docker/dockerfile:1.7
# Build:  scripts/docker-build.sh   (or: docker build -t openshell-annotate:dev .)
#
# `openshell-policy-schema` is a git dependency on https://github.com/NVIDIA/OpenShell (tag v0.1.7, commit pinned
# in Cargo.lock), fetched by cargo during the build, so no local OpenShell checkout is needed.
FROM rust:1.94-bookworm AS build
WORKDIR /build
# Tree-sitter grammars are C sources compiled by cargo's build scripts (gcc is in the rust image).
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release --locked && \
    strip target/release/openshell-annotate && \
    cp target/release/openshell-annotate /openshell-annotate

# distroless/cc: glibc + libgcc only (no shell, no package manager). Runs as root by default so the
# tool can write into a bind-mounted /src; use `docker run -u $(id -u):$(id -g)` to keep file ownership.
FROM gcr.io/distroless/cc-debian12
COPY --from=build /openshell-annotate /usr/local/bin/openshell-annotate
LABEL org.opencontainers.image.title="openshell-annotate" \
      org.opencontainers.image.description="Policy-as-code for OpenShell: @openshell source directives -> policy.yaml" \
      org.opencontainers.image.licenses="Apache-2.0"
WORKDIR /src
ENTRYPOINT ["/usr/local/bin/openshell-annotate"]
CMD ["--help"]
