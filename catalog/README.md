# Global problem catalog

`problems.json` contains shipped global problem metadata and language adapter source paths. It contains no problem-set order. `catalog_revision` controls one-time synchronization into the runtime database; bump it when shipped catalog data changes.

Custom problems are normally added through `practice problems add`. The database is authoritative for local CRUD. A later catalog export/import command can promote local entries into version-controlled distribution data.

## Statement content and provenance

Shipped statement briefs are independently written from checked-in titles, topics, Python and Rust interfaces, data structures, and public executable cases. They do not reproduce third-party problem prose. The executable adapter cases remain authoritative when a brief is intentionally narrow or a detail is not specified.

External URLs are references for users who want additional context. They are not sources for local statement text and are not required to use the catalog offline.
