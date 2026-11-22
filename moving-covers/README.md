# Moving covers

Moving covers plays an album's moving cover from Apple Music in the full player, in place of the still
cover, when Apple Music has one. The loop is silent.

It plays only while the full player shows the cover. On Android, it also needs nori in front, the screen
on and **Motion** set to **Full**. The terminal app and the iPod app show the still cover.

## What it sends

- To Apple's iTunes Search, the album's artist and name, to find the album in the US store. An album
  that is not in the US store has no moving cover.
- To the Apple Music catalog, the album it found, to ask for its moving cover.
- To music.apple.com, a request for the web player's page and scripts, which hold the key the catalog
  asks for. nori fetches the key again only when Apple stops taking it.

The loop itself comes from Apple's servers too, and is a few megabytes. nori remembers each album's
answer, and looks an album with no moving cover up again after a week. On Android, nori keeps the loops
it fetched, so each one downloads once.
