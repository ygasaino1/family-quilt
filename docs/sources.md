# Sources consulted (provenance)

| Source | Licence | Used for | How |
|---|---|---|---|
| GeneaQuilts repo (jdfekete/geneaquilt) | BSD (3-clause text) | feature inventory, visual encoding, layering behaviour (`docs/geneaquilts-analysis.md`) | behaviour read from code; **no code copied yet** |
| Owner's Node prototype (`family-tree.zip`) | owner's own | schema ideas, id generator, tests as behaviour checklist | behaviour ported to Rust, no code style copied |
| SQLCipher / `sqlcipher-src`, `rusqlite`, `libsqlite3-sys` | BSD-style / MIT | encryption, SQLite | dependencies |
| Gansner et al., "A Technique for Drawing Directed Graphs", 1993 | paper | network simplex ranking in `family-layout` | implemented from the paper, no code copied |
| Graphviz `dot` (EPL/CPL) | licence not needed: run as an external tool only | quality reference in `examples/layout_compare.rs` | development only, not linked, not shipped |
| Gramps QuiltView addon | GPL (to be checked) | planned behavioural reference | not used yet; no code copy |
| RFC 9562 (UUIDv7), RFC 4648 (base32), Argon2 (RFC 9106) | standards | ids, recovery key text, KDF | implemented from spec |
