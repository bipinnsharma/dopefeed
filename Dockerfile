FROM rust:1-slim-bookworm AS build
RUN apt-get update \
    && apt-get install -y --no-install-recommends cmake \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
COPY templates ./templates
COPY static ./static
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home /app dopefeed
WORKDIR /app
COPY --from=build /app/target/release/dopefeed /usr/local/bin/dopefeed
COPY static ./static
RUN mkdir -p /app/data && chown -R dopefeed:dopefeed /app
USER dopefeed
VOLUME ["/app/data"]
ENV BIND_ADDR=0.0.0.0:3000 \
    DATABASE_URL=sqlite://data/dopefeed.db?mode=rwc \
    BASE_URL=http://localhost:3000 \
    RUST_LOG=info
EXPOSE 3000
CMD ["dopefeed"]
