# Image de dev : toolchain Rust complète, aucune installation locale requise.
FROM rust:1-alpine
RUN apk add --no-cache musl-dev nodejs npm && rustup component add clippy rustfmt
COPY crates/moli-overkiz/test-fixtures/package*.json /opt/moli-mqtt-test/
RUN npm ci --prefix /opt/moli-mqtt-test --ignore-scripts --omit=dev
ENV MOLI_MQTT_TEST_AEDES=/opt/moli-mqtt-test/node_modules/aedes/aedes.js
ENV CARGO_TARGET_DIR=/target
WORKDIR /src
