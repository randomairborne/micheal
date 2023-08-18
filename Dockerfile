FROM rust:alpine AS builder

WORKDIR "/build"

COPY . .

RUN apk add opus alpine-sdk
RUN cargo build --release

FROM alpine

RUN apk add opus
COPY --from=builder /build/target/release/micheal /usr/bin/micheal

ENTRYPOINT "/usr/bin/micheal"
