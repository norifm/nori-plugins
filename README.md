# nori plugins

nori's own plugins, and its official plugin repo. A plugin is a WebAssembly component built with nori's
SDK (`crates/plugin-sdk` in nori; how they work: nori's `docs/developers/plugins.md`).

| plugin | what |
|---|---|
| `orbit` | a sound stage: the music circles your head |
| `playback` | speed, pitch, skipping silence and what previous does, as overrides of nori's values |
| `youtube-music` | YouTube Music as a provider: a kind of profile to add |

## The repo

`dist/` is the repo nori lists by default: one `.noriplugin` per plugin (a zip of `plugin.wasm`,
`strings/` and an optional `icon.svg`) and `index.json`, each plugin's id, name, about, version, plugin
API version, package and the package's sha256. `python3 tools/pack.py` builds every plugin and writes
both; publishing is committing `dist/`. Any other repo is the same shape on any static host, added in
nori by its `index.json`'s URL.
