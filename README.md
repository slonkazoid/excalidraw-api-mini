# excalidraw-api-mini

lightweight excalidraw sharing API implementation in rust

## Building

```sh
cargo build --release # current target
cargo zigbuild --target=x86_64-unknown-linux-musl # amd64 linux with static libc
```

## Usage

build the excalidraw frontend with the correct endpoints set:

```sh
export VITE_APP_BACKEND_V2_GET_URL=http://localhost:2799/ VITE_APP_BACKEND_V2_POST_URL=http://localhost:2799/
export VITE_APP_WS_SERVER_URL=…
export NODE_ENV=production
yarn build:app:docker # yes, :docker is the configuration you probably want
cd excalidraw-app/build
caddy file-server --listen :8000 --browse
```

start `excalidraw-api-mini`:

```sh
export DATABASE_URL="postgres:///excalidraw?host=%2Frun%2Fpostgresql&user=$USER"
export CORS_ORIGIN=http://localhost:8000
excalidraw-api-mini
```

## Gotchas

- SIGTERM (or the equivalent) will not gracefully shut down excalidraw-api-mini
  on non-UNIX systems, only SIGINT (or the equivalent) will.
- Only supported backend is PostgreSQL (which you should be using anyways).
- No per-client rate limiting, only a global concurrency limit to not bog down
  the database in case of a DOS attack.
- Shared items last forever (you should use an external script to manage
  this—that is why the `inserted_at` field exists.
