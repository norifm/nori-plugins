# nori plugins

This repository holds nori's own plugins and the plugin repo that nori lists by default. Each plugin is
a WebAssembly component built with nori's plugin SDK. To learn how plugins work and how to write one,
see [Plugins](https://github.com/norifm/nori/blob/master/docs/developers/plugins.md) in nori's developer
docs.

## The plugins

| Plugin | What it adds | Extension points |
|---|---|---|
| `mix-tuning` | AutoMix's longest mix, tempo change and parts, past what its style sets | `lifecycle` (overrides) |
| `moving-covers` | An album's moving cover from Apple Music, in the player | `slots` |
| `sony` | Battery, noise control and settings for Sony headphones | `device-control`, `hooks`, `lifecycle` |
| `soundcloud` | SoundCloud as a kind of profile | `provider` |
| `soundcore` | Battery, noise control and settings for Soundcore headphones | `device-control`, `hooks`, `lifecycle` |
| `youtube-music` | YouTube Music as a kind of profile | `provider` |

Each plugin's `README.md` is what listeners read on its page in nori.

Plugins for nori's developers (its checks' fixtures, examples, our own hardware) are in
[nori-dev-plugins](https://github.com/norifm/nori-dev-plugins).

## Build a plugin

The toolchain is pinned in `rust-toolchain.toml`, with the `wasm32-wasip2` target. To build one plugin
and pack it into `dist/`, run:

```sh
python3 tools/pack.py soundcore
```

`tools/pack.py` with no names packs every plugin, into `dist/<plugin>-<version>.noriplugin`. To try one,
install the file in nori with **Install from file** on the **Get plugins** page.

The SDK comes from nori's repository, at the commit that `Cargo.lock` names. To move it to nori's newest
commit, run `cargo update -p nori-plugin-sdk`. To build against your own nori checkout while you change
the SDK, run:

```sh
cargo build --config 'patch."https://github.com/norifm/nori".nori-plugin-sdk.path="../nori/crates/plugin-sdk"'
```

That command rewrites `Cargo.lock` to the local path. Do not commit that change.

## Release a plugin

1. In the plugin's `Cargo.toml`, set the new `version`, and commit.
2. Push the tag `<plugin>-<version>`, for example `soundcore-0.11.0`.

The release workflow (`.github/workflows/release.yml`) checks that the tag matches the plugin's
`Cargo.toml` and runs the plugin's tests. It then packs the plugin, makes a GitHub release with the
package, and commits the new version to `index.json` on `main`. Nobody writes `index.json` by hand.

Packing is deterministic. To check a release, pack its tag again and compare the sha256 with the one in
`index.json`.

`python3 tools/pack.py --links` points each plugin in `index.json` at its README and icon on `main`,
and builds nothing. Each plugin's `icon.svg` is its icon's source.

## The repo

nori reads `index.json` from this repository's `main` branch, and offers the newest version of each
plugin that its plugin API runs. Any static host can serve another repo of the same shape, and users add
it by the address of its `index.json`. The index format, signatures and API versions are in the
[plugin reference](https://github.com/norifm/nori/blob/master/docs/developers/plugin-reference.md).

## License

AGPL-3.0-or-later, as `Cargo.toml` says.
