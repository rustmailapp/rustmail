# RustMail UI

The RustMail web client: SolidJS + TypeScript + Tailwind CSS, built with Vite.

This UI is not deployed standalone. `make build` compiles it to `ui/dist`, which the `rustmail-api` crate embeds into the Rust binary via `rust-embed` and serves at the HTTP port.

## Package Manager

pnpm only (`pnpm-lock.yaml` is the sole lockfile).

```sh
pnpm install
```

## Development

Run the Vite dev server together with the Rust backend from the repository root:

```sh
make dev        # UI dev server + backend concurrently
make dev-ui     # UI dev server only (pnpm dev)
```

The dev server listens on `http://localhost:3000` and proxies `/api` and the `/api/v1/ws` WebSocket to the backend on `http://localhost:8025`, so a backend must be running for data to load.

## Scripts

```sh
pnpm dev        # Vite dev server on :3000
pnpm build      # type-check (tsc -b) then build to ui/dist
pnpm preview    # preview the production build
```

## Production Build

From the repository root:

```sh
make build
```

This runs `pnpm build` to produce `ui/dist`, then compiles the Rust binary with those assets embedded.
