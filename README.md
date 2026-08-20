<p align="center">
  <img src="./assets/readme/hero.svg" width="100%" alt="Osmos Core: content-addressable local history with a daemon API for clients">
</p>

# Osmos Core

[🇬🇧 English](README.md) · [🇹🇷 Türkçe](README.tr.md)

`osmos-core` is the Rust engine behind Osmos. It records a directory's local history in content-addressable storage, keeps repository metadata in SQLite, and exposes the work through a Unix-domain-socket daemon for client applications.

> Status: local versioning and the daemon API are implemented. Peer discovery and QUIC transport remain planned work.

## What is here

| Crate | Responsibility |
| --- | --- |
| [`osmos-core`](crates/osmos-core) | Detects changes; stores BLAKE3-addressed blobs; manages repositories, commits, and branches in SQLite. |
| [`osmos-daemon`](crates/osmos-daemon) | Accepts newline-delimited JSON over `/tmp/osmos-daemon.sock` and forwards commands to the engine. |
| [`osmos-transport`](crates/osmos-transport) | Reserved for a future mDNS + QUIC transport layer. |

## How it works

```text
working directory → BLAKE3-addressed blobs + SQLite metadata → daemon → local clients
```

An initialized repository receives an `.osmos/` directory:

```text
<repo_root>/.osmos/
├── meta.db       # repositories, commits, tree entries, branches
└── blobs/
    └── ab/cd…    # BLAKE3-addressed file content
```

## Get started

Requires Rust 1.75 or newer.

```bash
cargo build --release
cargo test
```

Run the local daemon:

```bash
cargo run --bin osmos-daemon
```

It listens at `/tmp/osmos-daemon.sock`.

## Daemon protocol

Requests and responses are newline-delimited JSON. For example, initialize a repository:

```json
{"id":"e43b1747-8cfb-4a5d-b2a8-12cd3111b7df","cmd":{"type":"init_repo","path":"/absolute/path/to/project","name":"My Project","mode":"client"}}
```

Supported commands include `ping`, repository initialization and status, commits, and branch operations (`create`, `list`, `switch`, `merge`, and `delete`). See [`crates/osmos-core/src/lib.rs`](crates/osmos-core/src/lib.rs) for the wire types.

## Roadmap

- [x] Local versioning, BLAKE3 blob storage, SQLite metadata, and the socket API.
- [ ] Peer discovery and synchronization through `osmos-transport`.

## License

MIT — see [LICENSE](LICENSE).
