SHELL := /bin/bash
port ?= 5499
url ?= postgres://root:password@localhost:$(port)/rainfrog?sslmode=disable
version ?= ""

.DEFAULT_GOAL := restart

dev:
	cargo run -- -u $(url)

dev-termux:
	cargo run --features termux --no-default-features -- -u $(url)

profile:
	cargo flamegraph --post-process flamelens --root -- -u $(url)

db-up:
	PORT=$(port) docker compose up -d --wait
	sleep .25

db-down:
	PORT=$(port) docker compose kill
	PORT=$(port) docker compose rm -f -v

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
	echo $(gh pr diff)
	@read -n 1 -p "are you sure you want to release v$(version)? [Y/n] " confirmation && if [ "$$confirmation" = "Y" ]; then echo " continuing..."; else echo " aborting..."; exit 1; fi
	gh pr merge --admin --squash --delete-branch
	git checkout main
	git pull
	git tag "v$(version)" main
	git push origin "v$(version)"
