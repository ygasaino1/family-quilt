# family-quilt

A local desktop app for a real family's genealogy: the **GeneaQuilts** matrix view (rows = people, columns = families), SQLite storage, non-biological parent links (adopted, foster, step, guardian, ...), an AI-agent-friendly review flow, and fast "how are A and B related?".

Status: **Phases 1 to 3 of 6 done (Gate B accepted)** (store layer). Specification: `docs/spec-prompt.md`. Decisions so far: `docs/decisions.md`. Analysis of the original: `docs/geneaquilts-analysis.md`.

## What exists today

| Crate | What it holds |
|---|---|
| `crates/store` (`family-store`) | schema + migrations, UUIDv7 ids, fuzzy dates, name search keys (any script), store layer with access context, revision history, in-memory graph snapshots, optional password protection (SQLCipher), recovery key, backups |

```
cargo test                     # 57 tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```
First build compiles SQLCipher and OpenSSL from source (about 2-3 minutes; needs a C compiler and Perl). `--no-default-features --features plain-only` builds without SQLCipher; protected trees are then refused with `EncryptionUnavailable` (never silently written unencrypted).

## Architecture (Phase 1)

* **Only the store layer writes SQL.** Every public function takes an `AccessContext` (v1: `Everything`) so per-vault encryption can later filter in one place (`docs/future-vaults.md`).
* Every change to a person, name, link or place writes a `revision` row (before/after JSON).
* Soft deletes only. Nothing is ever `DELETE`d.
* Rules enforced in one transaction and again by CHECK/UNIQUE constraints: no cycles, no duplicate links, both people must exist, partnerships stored once.
* Family derivation, layout and relationship finding (later phases) work on `Store::load_graph`, an in-memory snapshot with integer indexes, so hidden data can simply be absent.
* Errors carry a stable `code()` and `params()`; the UI translates them (spec section 20).

### Password protection (spec section 16)
Random 256-bit data key encrypts the file (SQLCipher). `<tree>.keys` stores that key wrapped by the password (Argon2id) and by a one-time recovery key (XChaCha20-Poly1305). **Without the password or the recovery key the data cannot be recovered.** Protected status is read from the file header; backups of a protected tree are protected and carry a copy of the key file. See `docs/decisions.md` (Phase 1, items 1-3) for the trade-offs.

## Dependencies and why

| Crate | Why |
|---|---|
| `rusqlite` (+ `bundled-sqlcipher-vendored-openssl`) | SQLite with no system dependency; SQLCipher for optional encryption; OpenSSL vendored so Windows needs no system copy |
| `serde`, `serde_json` | revision snapshots, key file, later the HTTP API |
| `thiserror` | error type with stable codes |
| `tracing` | logging (never logs secrets) |
| `uuid` | UUID type, formatting and parsing (the v7 generator with a strict counter is our own, `ids.rs`) |
| `getrandom` | OS randomness for ids, keys, salts |
| `argon2` | password key derivation (pure Rust) |
| `chacha20poly1305` | wrapping the data key (pure Rust) |
| `zeroize` | secrets are wiped from memory after use |
| `unicode-normalization` | name search keys (accents, Persian/Arabic letter variants) |
| `tempfile` (dev) | tests |

Build tools: C compiler and Perl (OpenSSL), dev only.

## Licence
New code: MIT (`LICENSE`). No GeneaQuilts code is ported yet; when it is, the BSD notice goes into `NOTICE`, file headers and the About screen. Sample `.ged` files from the original are not shipped.
