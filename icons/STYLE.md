# Instance icon style

Used by the launcher's instance icon pickers (`apps/app/src/components/InstanceIcon.vue`).
Original artwork of this project, in the spirit of the pixel black cat mascot with its purple outline.

- **Look:** flat pixel art, solid fills, bright colors. No glow or other filters, no gradients, no raster.
- **Grid:** `viewBox="0 0 64 64"`, a 16x16 grid of 4px cells. The art lives in the inner 14x14 cells and is
  centered; `shape-rendering="crispEdges"`.
- **Outline:** a one-cell outline in purple `#8b5cf6` around the whole silhouette (every empty cell touching
  the art, diagonals included), so icons read on dark and light backgrounds. The background is transparent.
- **Shading:** one lighter and one darker shade of the base hue per subject, as single-cell highlights, speckles
  or an edge row. Faces and holes in ink `#231d33`.
- **Subjects:** blocks and heads are filled squares; ingots, gems, items and the flame are shaped silhouettes.
  Subjects are generic Minecraft things; the modpack icons are a parcel and a puzzle piece.
- **Colors:** close to the Minecraft colors of the subject, a bit more saturated.

Main colors: grass `#6fd65a` on dirt `#9a6440`, stone `#a1a9b6`, planks `#e8c27a`, brick `#e0525e`,
TNT `#e8344a`, iron `#cfd6e4`, gold `#f5c531`, diamond `#3fdde8`, creeper `#5fd16a`, enderman `#2d2548`
with eyes `#e060ff`, skeleton `#e8ecf2`, fox `#ff7a35`, bee `#ffd95a`, ink `#231d33`.

The keys in `index.json` are identifiers (shared with imported instances) and must not change; files may be
renamed as long as every key still maps to one.

`icons/preview.html` shows the whole set on dark and light backgrounds.
