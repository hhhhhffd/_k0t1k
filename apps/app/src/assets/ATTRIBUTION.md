# Asset attribution

Source repository: https://github.com/PrismLauncher/PrismLauncher (branch `develop`),
base path `launcher/resources/`. Prism's `COPYING.md` states that code and resources
derived from MultiMC are Apache-2.0 (Copyright 2013-2021 MultiMC Contributors),
see http://www.apache.org/licenses/LICENSE-2.0 .

## instance-icons/

Pixel-art legacy MultiMC instance icons, saved as 128x128 PNG.
License: Apache-2.0 (MultiMC Contributors, via Prism Launcher). Minecraft-themed
artwork may also be subject to Mojang's brand and asset guidelines.

- From `multimc/128x128/instances/<name>_legacy.png`: chicken, creeper, enderpearl,
  flame (256x256 source), ftb-logo, gear, herobrine, infinity, magitech, meat,
  netherstar, skeleton, squarecreeper, steve. `ftb-glow` comes from
  `multimc/128x128/instances/ftb_glow.png`.
- Upscaled 32x32 -> 128x128 with nearest-neighbour (no 128px version exists), from
  `multimc/32x32/instances/<name>_legacy.png`: brick, diamond, dirt, gold, grass,
  iron, planks, stone, tnt.
- Upscaled 50x50 -> 100x100 and padded to 128x128 (no 128px version exists), from
  `multimc/50x50/instances/enderman_legacy.png`: enderman.

Left out: the newer vector icons in `multimc/scalable/instances/` (bee, fox and
redrawn versions of the above) are CC BY-SA 4.0 (ShareAlike), which is awkward for
a non-CC-licensed app; the loader logos (forge, liteloader) and the MultiMC/Prism
launcher logos are not useful as instance icons.

## brands/

Logos are trademarks of their respective owners and are used only to identify the
corresponding service.

| File | Source | License / status |
|------|--------|------------------|
| `modrinth.svg` | `launcher/resources/multimc/scalable/instances/modrinth.svg` | Modrinth trademark; Prism's copy is tagged CC BY-SA 4.0 |
| `ftb.svg` | `launcher/resources/multimc/scalable/instances/ftb_logo.svg` | Feed The Beast trademark; Prism's copy is tagged CC BY-SA 4.0 |
| `prism.svg` | `launcher/resources/multimc/scalable/instances/prismlauncher.svg` | Prism Launcher logo; CC BY-SA 4.0 (`program_info/LICENSE`: applies to logos and branding) |
| `curseforge.svg` | simple-icons v13.0.0 `icons/curseforge.svg` (https://github.com/simple-icons/simple-icons), recolored to CurseForge orange `#F16436` | Icon set is CC0-1.0; the CurseForge mark is a trademark of Overwolf |

Prism Launcher logo: (c) Prism Launcher contributors (AutiOne, Boba, ely, Fulmine,
gon sawa, Pankakes, tobimori, Zeke), CC BY-SA 4.0,
https://creativecommons.org/licenses/by-sa/4.0/ . The recolored/redistributed
SVGs here are unmodified except for the CurseForge fill color.
