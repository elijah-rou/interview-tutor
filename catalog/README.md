# Global problem catalog

`problems.json` contains metadata and Python/Rust adapter paths for the 79 shipped global problems. It contains no problem-set order; Blind 75 remains a separate 75-member set. `catalog_revision` controls one-time synchronization into the runtime database; bump it when shipped catalog data changes.

Custom problems are normally added through `practice problems add`. The database is authoritative for local CRUD. A later catalog export/import command can promote local entries into version-controlled distribution data.

## Statement content and provenance

Shipped statement briefs are independently written from checked-in titles, topics, Python and Rust interfaces, data structures, and public executable cases. They do not reproduce third-party problem prose. The local briefs together with the language signatures define the intended user contract. Checked-in executable cases are representative rather than exhaustive, and they are authoritative if a case conflicts with a brief.

External URLs are references for users who want additional context. They are not sources for local statement text and are not required to use the catalog offline.
