# Instance icon style

Used by the launcher's instance icon pickers (`apps/app/src/components/InstanceIcon.vue`).
The icon keys and subjects follow Prism Launcher's instance icons, so imported Prism instances keep theirs.
Every icon is drawn from scratch as SVG.

- **Look:** flat neon. No glow or other filters, no gradients, no white highlight, no raster.
- **Grid:** `viewBox="0 0 64 64"`, round caps and joins on the root `<svg>`.
- **Tile:** blocks and mobs are a front-facing tile, `rect x=8 y=8 w=48 h=48 rx=9`.
  The tile is filled with its color at `fill-opacity=".18"` and stroked with the same color, `stroke-width=3`.
- **Details:** faces, bands and caps (eyes, mouths, the grass top, the TNT band) are **solid fills** in bright
  colors. Texture lines (planks, bricks, iron bands, stone cracks) are strokes `3` in the tile color.
- **Non-tile subjects** (meat, ender pearl, nether star, flame, gear, magitech, FTB): a single silhouette
  with the same tinted fill and stroke, plus a solid detail.
- **Colors:** only the palette below; 1–3 colors per icon.

| Token | Hex | Used for |
|---|---|---|
| green | `#5dff6a` | grass, creeper |
| soil | `#ff9d3c` | dirt, hair |
| yellow | `#ffd23c` | gold, bee, beak, FTB |
| red | `#ff3d5a` | TNT, wattle, red eyes, FTB |
| brick | `#ff5a3c` | bricks |
| wood | `#ffb04a` | planks |
| stone | `#aebfdc` | stone |
| iron | `#dfe8ff` | iron, gear |
| cyan | `#3cf0ff` | diamond, bee and skeleton eyes |
| purple | `#c26bff` | enderman, magitech, FTB tile |
| magenta | `#ff4dff` | ender eyes |
| teal | `#2dffc4` | ender pearl |
| blue | `#3c8cff` | FTB |
| sky | `#3cc8ff` | gear ring |
| periwinkle | `#6a8cff` | Steve's eyes |
| paper | `#f4f4ff` | chicken, fox white, eye whites, TNT band |
| bone | `#e8f0ff` | skeleton |
| fire | `#ff7a2d` | fox, flame |
| meat | `#ff7a5a` | meat |
| skin | `#fff4b0` | Steve, Herobrine, nether star |

Known limitation: made for dark backgrounds. Icons in `iron`, `bone`, `paper` or `skin` lose contrast on light ones.
