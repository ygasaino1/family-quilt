# Phase 1 report

Verified on Linux (rustc 1.97): `cargo test` 57 passed, `cargo clippy --all-targets -- -D warnings` clean, `cargo fmt --check` clean, `--features plain-only` compiles and fails closed.

## Still to verify on Windows (not possible in the sandbox)
1. Install Rust (MSVC toolchain), Visual Studio Build Tools (C++), and **Strawberry Perl** (OpenSSL is compiled from source).
2. `cargo test` in the repo root. If the OpenSSL build fails, report the error: fallback is `bundled-sqlcipher` with a prebuilt OpenSSL (`OPENSSL_DIR`), or a different crypto provider.
3. Release build is `cargo build --release`; check that the exe has no extra DLLs (`dumpbin /dependents`).

## Acceptance (spec section 16 / phase 1)
| Requirement | Test |
|---|---|
| protected tree needs password after reopen | `protected_tree_needs_the_password_and_is_not_plain_sqlite` |
| wrong password rejected, delayed, file untouched | `wrong_password_is_slow_and_harmless` |
| not readable by plain SQLite (header, no key) | same test; also checked with Python's stock sqlite3: "file is not a database" |
| backup of protected tree is protected | `backup_of_a_protected_tree_is_protected_and_restorable` |
| unprotected trees unchanged | `unprotected_tree_is_plain_sqlite_and_persists` |
| set / change / remove password, recovery key | `protect_then_unprotect...`, `change_password...`, `recovery_key...` |
| `vault_id` on all tables, NULL | `every_table_has_vault_id_and_it_is_null` |
| cycles / duplicates rejected, migrations idempotent | `cycles_are_rejected`, `duplicates_...`, `schema::tests` |
| locked state blocks both doors | Phase 4 (HTTP does not exist yet); the store returns `PasswordRequired` |
