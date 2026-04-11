# CAVE Unified Runtime — Multi-stage build
# Target: ~50MB final image

FROM rust:1.85-alpine AS builder

RUN apk add --no-cache musl-dev openssl-dev openssl-libs-static

WORKDIR /build
COPY . .

RUN cargo build --release --bin cave-runtime

# Runtime image
FROM alpine:3.21

RUN apk add --no-cache ca-certificates

COPY --from=builder /build/target/release/cave-runtime /usr/local/bin/cave-runtime
COPY cave-runtime.yaml /etc/cave/cave-runtime.yaml

EXPOSE 8080 9090

ENTRYPOINT ["cave-runtime"]
CMD ["--config", "/etc/cave/cave-runtime.yaml"]
