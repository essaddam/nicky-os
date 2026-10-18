# The archive has its own binary/image: no Parquet dependency enters moli-os.
FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
ENV CARGO_BUILD_JOBS=1
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY locales locales
RUN cargo build --release --locked -p moli-archive
RUN mkdir -p /archive-data && chown 1000:1000 /archive-data

FROM scratch
ARG BUILD_SOURCE_SHA=unknown
LABEL io.moli.source-sha=$BUILD_SOURCE_SHA
COPY --from=build /src/target/release/moli-archive /moli-archive
COPY --from=build --chown=1000:1000 /archive-data /data/archive
USER 1000:1000
WORKDIR /data
ENV MOLI_ARCHIVE_DATA=/data/archive
ENTRYPOINT ["/moli-archive"]
CMD ["serve"]
