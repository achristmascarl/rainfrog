url ?= postgres://root:password@localhost:5432/rainfrog

dev:
	cargo run -- -u $(url)

profile:
	cargo flamegraph --post-process flamelens --root -- -u $(url)
