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
| `orbit` | A sound effect: the music circles your head | `sound-stage` |
| `playback` | Speed, pitch, skipping silence and what previous does | `lifecycle` (overrides) |
| `remote-switch` | A switch on **Playing on** that turns remote control and jams off | `lifecycle` (overrides) |
| `showcase` | One of every kind of row and place a plugin can fill, for plugin authors | `lifecycle`, `slots` |
| `sony` | Battery, noise control and settings for Sony headphones | `device-control`, `hooks`, `lifecycle` |
| `soundcloud` | SoundCloud as a kind of profile | `provider` |
| `soundcore` | Battery, noise control and settings for Soundcore headphones | `device-control`, `hooks`, `lifecycle` |
| `speaker` | Casting to a nori speaker on your network | `lifecycle`, `hooks` |
| `youtube-music` | YouTube Music as a kind of profile | `provider` |

Each plugin's `README.md` is what listeners read on its page in nori.

## Build a plugin

The toolchain is pinned in `rust-toolchain.toml`, with the `wasm32-wasip2` target. To build one plugin
and pack it into `dist/`, run:

```sh
python3 tools/pack.py orbit
```

`tools/pack.py` with no names packs every plugin. The result is `dist/<plugin>-<version>.noriplugin`, a
zip of `plugin.wasm`, `strings/`, and the plugin's `icon.png` and `README.md` when it has them. To try
it, install the file in nori with **Install from file** on the **Get plugins** page.

The SDK comes from nori's repository, at the commit that `Cargo.lock` names. To move it to nori's newest
commit, run `cargo update -p nori-plugin-sdk`. To build against your own nori checkout while you change
the SDK, run:

```sh
cargo build --config 'patch."https://github.com/norifm/nori".nori-plugin-sdk.path="../nori/crates/plugin-sdk"'
```

That command rewrites `Cargo.lock` to the local path. Do not commit that change.

## Release a plugin

1. In the plugin's `Cargo.toml`, set the new `version`, and commit.
2. Push the tag `<plugin>-<version>`, for example `orbit-0.8.0`.

The release workflow (`.github/workflows/release.yml`) checks that the tag matches the plugin's
`Cargo.toml` and runs the plugin's tests. It then packs the plugin, makes a GitHub release with the
package, and commits the new version to `index.json` on `main`. Nobody writes `index.json` by hand.

Packing is deterministic: the same tag packs to the same bytes on any machine. To check a release, pack
its tag again and compare the sha256 with the one in `index.json`.

`python3 tools/pack.py --links` points each plugin in `index.json` at its README and icon on `main`,
and builds nothing.

An icon is a PNG, 256 px square, one color on transparent, at most 256 KiB. nori draws it in the page's
ink. Each plugin's `icon.svg` is its icon's source.

## The repo

nori reads `index.json` from this repository's `main` branch. For each plugin, the index lists its id,
name, about line, source, README and icon. For each version, it lists the plugin API version, the
release day, the package's URL and its sha256. nori offers the newest version that its plugin API runs. It updates a plugin at most
once a day, from the repo the plugin came from, unless the user keeps that version.

Any static host can serve another repo of the same shape: an `index.json` and its packages. Their URLs
are absolute, or relative to the index. Users add such a repo by the address of its `index.json`.
nori warns them first. The index format, signatures and API versions are in the
[plugin reference](https://github.com/norifm/nori/blob/master/docs/developers/plugin-reference.md).

## License

AGPL-3.0-or-later, as `Cargo.toml` says.
