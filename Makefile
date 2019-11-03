build:
	cargo build --release

nobatch:
	target/release/redis-ping-pong-server

max:
	target/release/redis-ping-pong-server -m max --max 500

minmax:
	target/release/redis-ping-pong-server -m max --min 10 --max 500

build-debug:
	cargo build

run-debug:
	target/debug/redis-ping-pong-server

benchmark:
	redis-benchmark -n 100000 -t ping -c 1

.PHONY: build run build-debug run-debug

