# Global problem catalog

`problems.json` contains shipped global problem metadata and language adapter source paths. It contains no problem-set order. `catalog_revision` controls one-time synchronization into the runtime database; bump it when shipped catalog data changes.

Custom problems are normally added through `practice problems add`. The database is authoritative for local CRUD. A later catalog export/import command can promote local entries into version-controlled distribution data.
