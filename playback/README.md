# Speed and pitch

Speed and pitch changes how fast songs play and how high they sound. It can also skip silence in songs
and make previous always go back a song. nori plays by these values but has no settings for them.

## Settings

- **Speed** goes from 0.5× to 2×, in steps of 0.05. It starts at 1×.
- **Pitch** goes from 0.5× to 2×, in the same steps, apart from the speed. It starts at 1×.
- **Skip silence** skips the quiet parts of songs. It starts off.
- **Previous always goes back a song** starts off. Off, previous restarts the song once more than 3
  seconds of it played. On, previous goes to the song before.

## Good to know

- With **Skip silence** on, or a speed or pitch other than 1×, nori cannot use offload. The CPU decodes
  the music, which uses more battery.
- Nothing here changes your saved settings. Turn the plugin off, and nori's own values are back.
