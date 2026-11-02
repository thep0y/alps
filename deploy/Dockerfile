# syntax=docker/dockerfile:1
# The base images are arguments so local/CI environments can substitute
# mirrors. Defaults are the production bases.
ARG RUST_IMAGE=rust:1.99-bookworm
ARG RUNTIME_IMAGE=gcr.io/distroless/cc-debian12:nonroot

FROM ${RUST_IMAGE} AS builder
RUN apt-get update \
    && apt-get install -y --no-install-recommends cmake clang ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY examples ./examples
RUN cargo build --release --locked

FROM ${RUNTIME_IMAGE}
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=builder /src/target/release/alps /usr/local/bin/alps
ARG RUNTIME_USER=nonroot:nonroot
USER ${RUNTIME_USER}
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/alps"]
CMD ["--config", "/etc/alps/alps.toml"]
