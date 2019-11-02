build:
	cargo build --release

run:
	target/release/redis-ping-pong-server

build-debug:
	cargo build

run-debug:
	target/debug/redis-ping-pong-server

benchmark:
	redis-benchmark -n 100000 -t ping -c 1

.PHONY: build run build-debug run-debug

