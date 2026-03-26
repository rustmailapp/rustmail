.PHONY: dev dev-ui dev-server build build-ui build-server run check test lint fmt clean

# Development — run UI dev server + Rust backend concurrently
dev:
	@echo "Starting RustMail in dev mode..."
	@$(MAKE) -j2 dev-ui dev-server

dev-ui:
	cd ui && pnpm dev

dev-server:
	cargo watch -x 'run -- --db-path ./dev.db --http-port 8025 --smtp-port 1025'

dev-tui:
	cargo run -- tui

# Production build — UI first, then Rust binary with embedded assets
build: build-ui build-server

build-ui:
	cd ui && pnpm install --frozen-lockfile && pnpm build

build-server:
	cargo build --release

# Run the production binary
run: build
	./target/release/rustmail

# Quality checks
check:
	cargo check
	cd ui && pnpm exec tsc -b

test:
	cargo test

lint:
	cargo clippy --all-targets -- -D warnings
	cd ui && pnpm exec tsc -b

fmt:
	cargo fmt
	cd ui && pnpm exec prettier --write src/

# Cleanup
clean:
	cargo clean
	rm -rf ui/dist ui/node_modules
