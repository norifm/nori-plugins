# nori plugins

nori's own plugins, and its official plugin repo. A plugin is a WebAssembly component built with nori's
SDK (`crates/plugin-sdk` in nori; how they work: nori's `docs/developers/plugins.md`).

| plugin | what |
|---|---|
| `orbit` | a sound stage: the music circles your head |
| `playback` | speed, pitch, skipping silence and what previous does, as overrides of nori's values |
| `youtube-music` | YouTube Music as a provider: a kind of profile to add |
| `soundcloud` | SoundCloud as a provider: search and play, and your likes, playlists and stream when signed in |
| `showcase` | one of every place and kind of row a plugin can fill, for trying them out |
| `moving-covers` | an album's moving cover from Apple Music, in the player's sleeve |

## The repo

nori lists this repository by default. A plugin's versions are its GitHub releases, tagged
`<plugin>-<version>` (`orbit-0.2.0`), each with its `.noriplugin` (a zip of `plugin.wasm`, `strings/`
and an optional `icon.png` and `README.md`). `index.json` lists every plugin's id, name, about, source,
README and icon, and every version: its plugin API version, release day, package URL and sha256. nori offers the newest version it
can run, updates a plugin daily from the repo it came from unless it's held, and can go back to an older
version.

To release: set the version in the plugin's `Cargo.toml`, commit, and push the tag
`<plugin>-<version>`. The release workflow builds it with the pinned toolchain (`rust-toolchain.toml`),
packs it (`tools/pack.py`), makes the release and adds it to `index.json`; nobody writes the index by
hand; `tools/pack.py --links` points it at each plugin's README and icon. An icon is a 256 px PNG, one
color on transparent, which nori draws in the page's ink (`icon.svg` is its source). Packing is deterministic on any machine, so a package can be checked by packing its tag again and
comparing its sha256.

The SDK comes from nori's repository at the commit `Cargo.lock` names; `cargo update -p nori-plugin-sdk`
moves it to nori's newest. To build against a nori checkout of your own while changing the SDK:

    cargo build --config 'patch."https://github.com/norifm/nori".nori-plugin-sdk.path="../nori/crates/plugin-sdk"'

That rewrites `Cargo.lock` to the local path; don't commit it.

Another repo is the same shape on any static host: an `index.json` and its packages (URLs absolute, or
relative to the index). nori adds it by the index's URL, after a warning.

`python3 tools/pack.py [name ...]` packs into `dist/` without releasing, for a `.noriplugin` to install
by hand.
