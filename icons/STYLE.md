# Instance icon style

Used by the launcher's instance icon pickers (`apps/app/src/components/InstanceIcon.vue`).

- **Look:** flat line art in bright colors, no fills, no gradients, no glow or other filters, no raster. Not pixel art.
- **Grid:** `viewBox="0 0 64 64"`, artwork within roughly 8–56, centered.
- **Stroke:** two passes of the same geometry:
  1. colored stroke `2.6`, round caps and joins (no glow);
  2. white highlight on top: `stroke="#fff" stroke-opacity=".7" stroke-width=".9"`.
- **Corners:** frames use `rx` 3–6; everything else gets its rounding from round joins.
- **Blocks:** isometric cube. Top `32,9 54,21 32,33 10,21`, left face `10,21 32,33 32,57 10,45`,
  right face `32,33 54,21 54,45 32,57`. Texture is drawn with lines on the faces in face coordinates.
- **Mobs:** front-facing head in a rounded square (about 40×40), eyes and mouth from simple rects and lines.
- **Items / meta:** a single silhouette, plus at most one or two small accent details (sparkles, bubbles).
- **Colors:** each icon uses 1–3 colors from the palette below, never anything else.

| Token | Hex | Used for |
|---|---|---|
| green | `#5dff6a` | grass, creeper, plants |
| soil | `#ff9d3c` | dirt, bee stripes |
| yellow | `#ffd23c` | gold, sparks, star, accents |
| red | `#ff3d5a` | TNT, heart, wattle |
| brick | `#ff5a3c` | bricks |
| wood | `#ffb04a` | planks, handles, chest |
| stone | `#aebfdc` | stone |
| iron | `#dfe8ff` | iron, tool heads |
| cyan | `#3cf0ff` | diamond, sword |
| purple | `#c26bff` | obsidian, ender, book, default |
| magenta | `#ff4dff` | ender eyes, potion, magic |
| teal | `#2dffc4` | ender pearl |
| blue | `#3c8cff` | server |
| white | `#ffffff` | eyes, TNT band |
| paper | `#f4f4ff` | glass, pages, white fur |
| bone | `#e8f0ff` | skeleton |
| fire | `#ff7a2d` | flame, fox |
| amber | `#ffb03c` | beak |
| starlight | `#fff4b0` | star sparkles, shine |

Known limitation: the style is made for dark backgrounds. Icons drawn mostly in
`iron`, `bone`, `paper` or `white` lose contrast on light backgrounds.
