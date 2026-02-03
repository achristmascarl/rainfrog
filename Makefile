SHELL := /bin/bash
pg_port ?= 5499
mysql_port ?= 3317
oracle_port ?= 1521
postgres_url ?= postgres://root:password@localhost:$(pg_port)/rainfrog?sslmode=disable
mysql_url ?= mysql://root:password@localhost:$(mysql_port)/rainfrog?useSSL=false
sqlite_url ?= sqlite://./dev/rainfrog.sqlite3
oracle_url ?= jdbc:oracle:thin:rainfrog/password@localhost:$(oracle_port):rainfrog
duckdb_url ?= ./dev/rainfrog.db
url ?= $(postgres_url)
version ?= ""

.DEFAULT_GOAL := restart

.PHONY: dev profile restart release
dev:
	cargo run -- -u $(url)

dev-postgres:
	cargo run -- -u $(postgres_url)

dev-mysql:
	cargo run -- -u $(mysql_url)

dev-sqlite:
	cargo run -- -u $(sqlite_url)

dev-oracle:
	cargo run -- -u $(oracle_url)

dev-duckdb:
	cargo run -- -u $(duckdb_url) --driver duckdb

dev-termux:
	cargo run --features termux --no-default-features -- -u $(url)

dev-termux-sqlite:
	cargo run --features termux --no-default-features -- -u $(sqlite_url)

profile:
	cargo flamegraph --post-process flamelens --root -- -u $(url)

db-up: duckdb-up ci-db-up

duckdb-up:
	duckdb ./dev/rainfrog.db < ./dev/duckdb_init.sql

ci-db-up:
	sqlite3 ./dev/rainfrog.sqlite3 < ./dev/sqlite_init.sql
	PG_PORT=$(pg_port) MYSQL_PORT=$(mysql_port) ORACLE_PORT=$(oracle_port) docker compose up -d --wait
	sleep 1

db-down:
	rm -f ./dev/rainfrog.sqlite3
	rm -f ./dev/rainfrog.db
	docker compose kill
	docker compose rm -f -v

restart: db-down db-up dev

release:
	@if [ -z "$(version)" ]; then echo "version is required"; exit 1; fi
	git checkout main
	git pull
	@if [ $$(git tag -l "v$(version)") ]; then echo "version already exists"; exit 1; fi
	git checkout -b release/v$(version) && git push -u origin release/v$(version)
	sed -i "" "s/^version = .*/version = \"$(version)\"/" ./Cargo.toml
	cargo build
	git status
	git add Cargo.toml
	git add Cargo.lock
	git commit -m "bump to v$(version)"
	git push
	gh pr create --fill --label no-release-notes
	gh pr diff | cat
	@read -n 1 -p "are you sure you want to release v$(version)? [Y/n] " confirmation && if [ "$$confirmation" = "Y" ]; then echo " continuing..."; else echo " aborting..."; exit 1; fi
	gh pr merge --admin --squash
	git checkout main
	git pull
	git tag "v$(version)" main
	git push origin "v$(version)"
