# Bundled plugin baselines

`plugin.json` and the files it lists are the latest signed GitHub releases of the plugins, fetched by
`scripts/fetch-plugins.mjs` at the start of `pnpm build` (run it by hand with `pnpm fetch-plugins`). The script
verifies each release like the launcher does (Ed25519 signature, id, api, minLauncher, sha256) and keeps the
existing copy, with a warning, if the network is down or a release does not verify. Bundled copies are covered by
the signature of the launcher exe, so they carry no `plugin.json.sig`.

| Plugin   | Repository                  |
|----------|-----------------------------|
| ratscan  | hhhhhffd/ratka_checker      |
| packconv | hhhhhffd/conventor-mc-rp    |
