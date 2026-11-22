# Soundcore

Soundcore shows your Soundcore earbuds' or headphones' battery and noise control in nori, and lets you
change their settings from nori. It works on Android, and in the desktop app and the terminal app on
Linux and macOS.

The plugin was tested on the Liberty 4 NC. nori knows about 40 Soundcore models, among them the Liberty,
Life, Space and P series. Each model shows only the settings it has, and on other models unexpected
things may happen.

## Where it shows

- The battery shows in the player's output row.
- Noise control is on the **Playing on** sheet. On Android, while music plays on the headphones, you can
  also tap the glyph beside the seal in the player.
- The headphones' page holds **Noise control** and the **Touch controls**, **Sound** and **Device**
  pages.

## Good to know

- The headphones talk to one app at a time. Close the Soundcore app on this device, and on any other
  device that the headphones use.
- When the headphones connect, nori sets their equalizer flat and turns HearID off, so it does not
  stack with nori's.
- nori talks to the headphones over Bluetooth only and sends nothing to the internet. On Android, it asks
  for the Nearby devices permission.

Based on [github.com/Oppzippy/OpenSCQ30](https://github.com/Oppzippy/OpenSCQ30) by Kyle Scheuing
(GPL-3.0-or-later).
