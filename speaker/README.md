# nori speaker

Plays on a nori speaker on your network: an ESP32-S31 board running nori's speaker firmware.

Open the devices sheet and it looks for speakers; "Play on the speaker" starts and stops the cast, on the one it found last. nori sends the music in bursts, so the phone sleeps between them. The speaker's button plays, pauses and skips, and the volume keys set its volume.
