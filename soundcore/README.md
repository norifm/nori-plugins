# Soundcore

Soundcore shows your Soundcore earbuds' or headphones' battery and noise control in nori, and lets you
change their settings from nori.

The plugin was tested on the Liberty 4 NC. nori knows about 40 Soundcore models, among them the Liberty,
Life, Space and P series. Each model shows only the settings it has. Headphones it does not know are left
alone. On models other than the Liberty 4 NC, unexpected things may happen.

## Where it shows

- The battery shows in the player's output row.
- Noise control is on the **Playing on** sheet. On Android, while music plays on the headphones, the
  glyph beside the seal in the player also changes it when you tap it.
- The headphones' page holds **Noise control** and the **Touch controls**, **Sound** and **Device**
  pages.

## Good to know

- The headphones talk to one app at a time. Close the Soundcore app on this device, and on any other
  device that the headphones use.
- When the headphones connect, nori sets their equalizer to 0 dB on every band and turns HearID off.
  Their equalizer then does not stack with nori's.
- nori talks to the headphones over Bluetooth only and sends nothing to the internet. On Android, it asks
  for the Nearby devices permission.
- It works on Android, and in the desktop app and the terminal app on Linux and macOS.

Based on [github.com/Oppzippy/OpenSCQ30](https://github.com/Oppzippy/OpenSCQ30) by Kyle Scheuing
(GPL-3.0-or-later).
