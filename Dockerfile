FROM rust:1.67.0 as builder
WORKDIR /zkill-ws-importer/source
COPY /Cargo.toml .
COPY /Cargo.lock .
COPY /src ./src
RUN cargo install --locked --path .

FROM rust:1.67.0
WORKDIR /zkill-ws-importer
COPY --from=builder /zkill-ws-importer/source/target/release/zkill-ws-importer /zkill-ws-importer
COPY /prod/prod_config.json config.json

ENTRYPOINT  ["./zkill-ws-importer", "config.json"]
