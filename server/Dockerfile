FROM rust:1-bookworm AS build
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN useradd --uid 10001 --create-home extend \
    && mkdir /data && chown extend:extend /data
COPY --from=build /build/target/release/extend-computer-server /usr/local/bin/
USER extend
ENV EXTEND_BIND=0.0.0.0:8080 EXTEND_DATABASE=/data/extend.sqlite3
VOLUME /data
EXPOSE 8080
CMD ["extend-computer-server"]
