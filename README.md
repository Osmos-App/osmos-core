# Osmos Core

Osmos Core is a lightweight, content-addressable local version control engine and daemon written in Rust. It is designed to track directory changes, manage local branches, deduplicate files using BLAKE3 hashing, and expose a Unix domain socket API for client applications (such as Swift-based desktop or mobile apps).

---

## Architecture

The project is structured as a Rust workspace with three sub-crates:

1. **[`osmos-core`](crates/osmos-core)**: The core library containing:
   - File status detection and change tracking.
   - Content-addressable storage (blobs directory) with BLAKE3 deduplication.
   - SQLite-backed metadata tracking (`meta.db`) for repositories, branches, and commits.
   - A branch merge resolver using last-write-wins (LWW) resolution based on commit timestamps.
2. **[`osmos-daemon`](crates/osmos-daemon)**: A background daemon that:
   - Listens on a Unix domain socket (`/tmp/osmos-daemon.sock`).
   - Accepts newline-delimited JSON requests.
   - Forwards commands to `osmos-core` asynchronously.
3. **[`osmos-transport`](crates/osmos-transport)** (Work in Progress):
   - A placeholder crate designed for Phase 2, which will handle peer discovery via mDNS and P2P synchronization using QUIC.

---

## Layout on Disk

When a repository is initialized at `<repo_root>`, the engine creates a `.osmos/` directory:

```text
<repo_root>/.osmos/
  ├── meta.db       # SQLite database containing repos, commits, tree entries, and branches
  └── blobs/        # Content-addressable directory storage
      └── ab/       # Two-character prefix directory
          └── cd... # Blob file named by BLAKE3 hash
```

---

## Getting Started

### Prerequisites

- Rust (MSRV 1.75+)

### Compilation & Testing

Build the workspace:
```bash
cargo build --release
```

Run all unit tests and doc-tests:
```bash
cargo test
```

### Running the Daemon

You can start the background socket daemon by running:
```bash
cargo run --bin osmos-daemon
```
The daemon will create a Unix domain socket at `/tmp/osmos-daemon.sock` and output logging to the console.

---

## API Protocol Reference

The daemon communicates over the Unix domain socket using newline-delimited JSON.

### Request Format
```json
{
  "id": "e43b1747-8cfb-4a5d-b2a8-12cd3111b7df",
  "cmd": {
    "type": "init_repo",
    "path": "/absolute/path/to/my-project",
    "name": "My Project",
    "mode": "client"
  }
}
```

### Response Format
```json
{
  "id": "e43b1747-8cfb-4a5d-b2a8-12cd3111b7df",
  "ok": true,
  "data": {
    "repo_id": "c622b3e8-5b12-4217-91a5-81d3ee24a480",
    "name": "My Project",
    "root_path": "/absolute/path/to/my-project",
    "mode": "client",
    "created_at": "2026-06-26T11:30:00Z"
  }
}
```

### Supported Commands

- `ping`: Ping the daemon. Returns `{"pong": true}`.
- `init_repo`: Create a new Osmos repository at a specific path.
- `list_repos`: List repositories registered in the global configuration database.
- `get_status`: Retrieve the file change status of a repository (untracked, modified, staged, etc.).
- `create_commit`: Commit staged changes.
- `list_commits`: List commits in a repository or branch.
- `create_branch`: Create a new branch.
- `list_branches`: List branches in a repository.
- `switch_branch`: Switch the active branch of a repository.
- `merge_branch`: Merge another branch into the active branch.
- `delete_branch`: Delete a branch.
- `get_current_branch`: Get the currently checked-out branch.

---

## Current Status & Roadmap

- [x] **Phase 1: Local Versioning & Daemon API** — SQLite tracking, BLAKE3 blob storage, merge resolution, and socket API handler are fully implemented and tested.
- [/] **Phase 2: Peer-to-Peer Transport** — Discovery via mDNS and background synchronization via QUIC (`osmos-transport` crate) are currently in the planning/placeholder phase.

---

## License

This project is licensed under the MIT License. See [LICENSE](LICENSE) for details.
