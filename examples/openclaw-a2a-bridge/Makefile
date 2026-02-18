.PHONY: demo test launch stop clean build

BINARY ?= ../../target/debug/arkavo

build:
	cargo build --features kas --manifest-path ../../Cargo.toml

demo: build launch
	./demo.sh
	./stop.sh

test: build launch
	./test-bridge.sh
	./stop.sh

launch:
	./launch.sh

stop:
	./stop.sh

clean: stop
	rm -rf results/*.md logs/
