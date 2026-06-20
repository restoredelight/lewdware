# docs

This is the code for [Lewdware's website](https://lewdware.net), built using
Astro and Starlight and hosted on Cloudflare Pages. View it locally by running
`pnpm dev`.

## Lua Mode API

The mode API page is generated from `../shared/src/lua/api.lua` using
`src/lib/parse-luacats.ts` and `src/pages/reference/lua-api.astro`.

## Version manifests

This site hosts the generated version manifests (`src/data/latest.json` and
`src/data/pack-editor-latest.json`), allowing programs to query the latest
version of Lewdware and the pack editor, and to get the correct download link.
