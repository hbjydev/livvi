# syntax=docker/dockerfile:1

FROM rust:1.96.0 AS builder

WORKDIR /usr/src/livvi

COPY Cargo.toml Cargo.lock ./
COPY livvi-core/Cargo.toml livvi-core/
COPY livvi-core-macros/Cargo.toml livvi-core-macros/
COPY livvi-daemon/Cargo.toml livvi-daemon/
COPY livvi-discord/Cargo.toml livvi-discord/
COPY livvi-lcm/Cargo.toml livvi-lcm/
COPY livvi-memini/Cargo.toml livvi-memini/
COPY livvi-openai/Cargo.toml livvi-openai/
COPY livvi-store/Cargo.toml livvi-store/
COPY livvi-web/Cargo.toml livvi-web/
RUN mkdir -p \
    livvi-core/src \
    livvi-core-macros/src \
    livvi-daemon/src \
    livvi-discord/src \
    livvi-lcm/src \
    livvi-memini/src \
    livvi-openai/src \
    livvi-web/src \
    livvi-store/src && \
  echo "fn main() {}" > livvi-discord/src/main.rs && \
  echo > livvi-core/src/lib.rs && \
  echo > livvi-core-macros/src/lib.rs && \
  echo > livvi-daemon/src/lib.rs && \
  echo > livvi-lcm/src/lib.rs && \
  echo > livvi-memini/src/lib.rs && \
  echo > livvi-openai/src/lib.rs && \
  echo > livvi-web/src/lib.rs && \
  echo > livvi-store/src/lib.rs

RUN cargo fetch --locked

COPY . .
RUN cargo build --release --locked --offline -p livvi-daemon

FROM rockylinux/rockylinux:10-ubi

RUN dnf install -y ca-certificates openssl-libs && \
    dnf clean all && \
    rm -rf /var/cache/dnf

RUN groupadd --system livvi && \
    useradd --system --gid livvi --home-dir /app --shell /sbin/nologin livvi

COPY --from=builder /usr/src/livvi/target/release/livvi-daemon /usr/local/bin/livvi-daemon

USER livvi

ENTRYPOINT ["/usr/local/bin/livvi-daemon"]
