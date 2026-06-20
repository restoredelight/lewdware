# lw

`lw` is a CLI tool, primarily used to create, edit and build mode files.

## Mode file structure

A mode file consists of (in order) a fixed-size header, the ZSTD-compressed
Lua files, and binary-encoded metadata.
