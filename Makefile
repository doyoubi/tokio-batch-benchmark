build:
	cargo build --release

nobatch:
	RUST_LOG=redis_ping_pong_server=info target/release/redis-ping-pong-server

max:
	RUST_LOG=redis_ping_pong_server=info target/release/redis-ping-pong-server -m max --buf 50 --max 20

minmax:
	RUST_LOG=redis_ping_pong_server=info target/release/redis-ping-pong-server -m minmax --buf 50 --min 20 --max 500

build-debug:
	cargo build

run-debug:
	target/debug/redis-ping-pong-server

benchmark:
	redis-benchmark -n 10000000 -t ping -c 32 -P 16000

.PHONY: build run build-debug run-debug

