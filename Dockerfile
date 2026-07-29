FROM rust:1-bookworm AS build
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /app/target/release/infrabot /usr/local/bin/infrabot
USER 65532:65532
EXPOSE 8787
ENTRYPOINT ["/usr/local/bin/infrabot"]
