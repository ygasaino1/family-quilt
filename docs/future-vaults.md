# Future: per-vault encryption (NOT implemented, only prepared)

**Idea.** A *vault* is a group of owners who share a part of the tree. Each vault has its own random key, wrapped separately for every authorised user (same wrapping idea as the key file). Records in a vault carry `vault_id`; everything else is public (`vault_id IS NULL`, all of v1).

**Seams that exist now**
* `vault_id` on every table except `meta`, always NULL in v1 (test: `every_table_has_vault_id_and_it_is_null`).
* All SQL lives in `crates/store`; every function takes an `AccessContext` (`AccessContext::row_filter` is the single place to filter rows).
* Derivation, layout and path-finding read `Store::load_graph` snapshots, so hidden people/links are simply missing. `Graph::incomplete` (and the planned relationship-output flag) says "some data is private"; always false in v1.

**Open problems**
1. Links between a private and a public person: who may see the link, and does it reveal the private person exists?
2. Answers that pass through hidden people ("how are A and B related?") are wrong or incomplete; the output must say so.
3. Revoking access: the old key stays known to the revoked user; real revocation means re-encrypting the vault with a new key.
4. A plain file cannot enforce write permissions; anyone with the file and a key can write.
5. Metadata leaks: row counts, ids (UUIDv7 contain creation time), revision timestamps, file sizes.
6. `revision` and `change` rows hold copies of records and must inherit the vault of their target.

Multi-user editing is a separate sync/server project; nothing here attempts it.
