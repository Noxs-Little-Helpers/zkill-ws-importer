FROM rust:1.65.0 as builder

WORKDIR /zkill-ws-importer/source

COPY /Cargo.toml .
COPY /Cargo.lock .
COPY /src ./src
#RUN cargo build --release

#WORKDIR /zkill-ws-importer
#COPY /working_dir/config.json .



RUN cargo install --path .
FROM debian:buster-slim
RUN apt-get update && apt-get install -y extra-runtime-dependencies && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/zkill-ws-importer /usr/local/bin/zkill-ws-importer
COPY /working_dir/config.json config.json
#WORKDIR /zkill-ws-importer
ENTRYPOINT  ["zkill-ws-importer", "config.json"]
