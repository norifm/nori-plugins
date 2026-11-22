# nori speaker

nori speaker plays your music on a nori speaker on your network: a speaker that runs nori's speaker
firmware.

Open the **Playing on** sheet, and nori looks for speakers on your network while the sheet is open.
Choose a speaker, and the music plays there.

## How it plays

- nori sends the speaker up to a minute of music at a time, then sleeps until the speaker asks for more.
  The phone's Wi-Fi and CPU rest between those bursts.
- A pause or a seek is heard at once, even with a minute of music on the speaker.
- The speaker's buttons play, pause and skip. On Android, the phone's volume keys set the speaker's
  volume.

## Good to know

- The speaker must be on the same network as nori. A network that blocks devices from finding each other
  hides it.
- nori fetches each song as it always does, then passes it to the speaker over your network. Nothing goes
  to the internet for the speaker.
