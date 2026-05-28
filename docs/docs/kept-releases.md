---
title: Kept releases
sidebar_position: 7
---

# Kept releases

Some releases aren't a series. A Shonen Jump guidebook, an artbook,
a single-issue databook — none of those should sit in the review
queue waiting for a series match that doesn't exist, but it's wrong
to reject them too. The **kept** disposition is the resting place
for these: a small, browsable list of standalone items the operator
chose to hold onto.

Kept releases live at [`/admin/kept`](http://127.0.0.1:8080/admin/kept).
They don't appear in the browse feed (which is series-keyed) and
they don't show up in the review queue. They're also skipped by the
resolver — neither cron polls nor `retry-all` move them.

![Kept releases page](/img/screenshots/admin/kept.png)

## Marking a release as kept

From the [review queue](./review-queue.md): the **Keep** button on
any card. The release transitions to `resolution_status = standalone`
and leaves the queue immediately. There's no provider mapping
created; the release stays linked to whatever source feed it came
from.

That's the only entry point. There's no API for marking already-
resolved releases as kept — once a release auto-resolved into a
series, the right path is **Move** on the series detail page if the
match was wrong, then **Keep** when the moved-from row falls into
ambiguous.

## What the Kept page shows

Each card surfaces enough to identify the release without opening
the source page:

- **Title** — links to the source post.
- **Badges** — source kind + name, every detected format, byte size
  if known.
- **Magnet / .torrent / download** — whichever the source captured.
- **External links** — anything the source extracted from the post
  body (MangaUpdates, AniList, MAL, MangaDex). Useful even on a
  standalone if it links a related series.
- **Description** — expandable when the source captured one.
- **File list** — same display as the review queue, so you can
  confirm the contents without downloading.

Pagination is 20 items per page.

## Re-resolve

Each card has a **Re-resolve** button. It calls `POST /releases/{id}/retry`,
which sends the release back through the full resolution pipeline.
Useful when:

- A new provider was added that does carry the item as a series.
- The MangaBaka offline cache was refreshed and now contains a
  matching record.
- The item was marked kept by mistake.

A successful re-resolve removes the release from the Kept list (it
either auto-resolves into a series or lands in the review queue,
depending on what the pipeline finds).

## API

The Kept page is backed by `GET /releases?status=standalone`. The
`keep` action is `POST /releases/{id}/keep`; `Re-resolve` is the
same `POST /releases/{id}/retry` that the review queue uses.
