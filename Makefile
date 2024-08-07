port ?= 5499
url ?= postgres://root:password@localhost:$(port)/rainfrog?sslmode=disable

.DEFAULT_GOAL := restart

dev:
	cargo run -- -u $(url)

profile:
	cargo flamegraph --post-process flamelens --root -- -u $(url)

db-up:
	PORT=$(port) docker compose up -d --wait
	sleep .25

db-down:
	PORT=$(port) docker compose kill
	PORT=$(port) docker compose rm -f -v

restart: db-down db-up dev
