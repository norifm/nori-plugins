# Sony

Sony shows your Sony headphones' battery and noise control in nori, and lets you change their settings
from nori.

The plugin was tested on the WH-1000XM6. nori offers it WH-1000XM, WF-1000XM, LinkBuds and ULT WEAR
headphones, and any that list Sony's own Bluetooth service. It takes those that speak Sony's newer
protocol and leaves the rest alone. Each model shows only the settings it reports, and on models other
than the WH-1000XM6, unexpected things may happen.

## Where it shows

- The battery shows in the player's output row.
- Noise control is on the **Playing on** sheet. On Android, while music plays on the headphones, the
  glyph beside the seal in the player also changes it when you tap it.
- The headphones' page holds **Noise control** and the **Controls**, **Sound** and **Device** pages.

## Good to know

- The headphones talk to one app at a time. Close Sony's Sound Connect app while you use nori.
- When the headphones connect, nori turns their own equalizer off, so it does not stack with nori's.
- nori talks to the headphones over Bluetooth only and sends nothing to the internet. On Android, it asks
  for the Nearby devices permission.
- It works on Android, and in the desktop app and the terminal app on Linux and macOS.

The protocol comes from
[github.com/mos9527/SonyHeadphonesClient](https://github.com/mos9527/SonyHeadphonesClient) by mos9527 and
its contributors (MIT) and from
[codeberg.org/Freeyourgadget/Gadgetbridge](https://codeberg.org/Freeyourgadget/Gadgetbridge)
(AGPL-3.0-or-later).
