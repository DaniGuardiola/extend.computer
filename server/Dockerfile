FROM rust:1-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*
RUN useradd --uid 10001 --create-home extend \
    && mkdir /data && chown extend:extend /data
COPY --from=build /build/target/release/extend-computer-server /usr/local/bin/
USER extend
ENV EXTEND_BIND=0.0.0.0:8080 EXTEND_DATABASE=/data/extend.sqlite3
VOLUME /data
EXPOSE 8080
CMD ["extend-computer-server"]
