url ?= postgres://root:password@localhost:5432/rainfrog

.DEFAULT_GOAL := restart

dev:
	cargo run -- -u $(url)

profile:
	cargo flamegraph --post-process flamelens --root -- -u $(url)

db-up:
	docker compose up -d --wait

db-down:
	docker compose kill
	docker compose rm -f -v

dev-db:
	cargo run -- -u "postgres://root:password@localhost:5432/rainfrog" 

restart: db-down db-up dev-db
