# converter

Converts [Edgeware / Edgeware++](https://github.com/araten10/EdgewarePlusPlus)
packs into the pieces a Lewdware `.lwpack` needs: a tagged media list, pack
metadata, and a `behaviour.json` `Content` + `Experience` section.

It handles both the modern `index.json` layout and the legacy
`captions.json` / `media.json` / `prompt.json` / `web.json` layouts, and reads
from either a directory or a zip (`DirSource` / `ZipSource`). `corruption.json`
becomes a transition timeline and `config.json` becomes frequency anchors.

The pack editor uses this crate to offer Edgeware pack imports. `src/bin/dev-convert`
is a development helper for running a conversion from the command line.
