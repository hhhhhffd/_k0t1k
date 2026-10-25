# Instance icon style

Used by the launcher's instance icon pickers (`apps/app/src/components/InstanceIcon.vue`).
The keys, subjects and layout follow Prism Launcher's instance icons (see `apps/app/src/assets/ATTRIBUTION.md`).

- **Look:** flat, solid fills, bright colors. No glow or other filters, no gradients, no raster.
- **Grid:** `viewBox="0 0 64 64"`.
- **Tile:** blocks and mobs are a front-facing tile, `rect x=8 y=8 w=48 h=48 rx=8`, filled with the base color.
  Details are clipped to the tile (`clipPath id="t"`).
- **Details:** faces and holes in ink `#231d33`; texture as lighter/darker patches or 3px lines of the base hue;
  a light band along the top edge where the block has a top (grass, planks, skeleton, chicken).
- **Non-tile subjects** (meat, ender pearl, nether star, flame, gear, magitech, FTB): one solid silhouette
  with one or two details.
- **Colors:** close to the Minecraft colors of the subject, a bit more saturated.

Main colors: grass `#6fd65a` on dirt `#9a6440`, stone `#a1a9b6`, planks `#e8c27a`, brick `#e0525e`,
TNT `#e8344a`, iron `#e6ebf3`, gold `#f5c531`, diamond `#3fdde8`, creeper `#5fd16a`, enderman `#1f1a2c`
with eyes `#d85cff`, skeleton `#e8ecf2`, fox `#ff7a35`, bee `#ffd95a`, white `#f4f4ff`, ink `#231d33`.

`icons/preview.html` shows the whole set on dark and light backgrounds.
