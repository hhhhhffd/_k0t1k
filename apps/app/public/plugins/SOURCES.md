# Bundled plugin baselines

Built with `cargo build --release --target wasm32-unknown-unknown` and `wasm-bindgen --target web` (0.2.126) from a
clean `git archive HEAD` of the plugin repositories. Bundled copies are covered by the signature of the launcher exe,
so they carry no `plugin.json.sig`. A signed release from GitHub replaces them when it is newer.

| Plugin   | Repository                           | Commit                                   |
|----------|--------------------------------------|------------------------------------------|
| ratscan  | hhhhhffd/ratka_checker               | 9e79ab9c2275054da94bbc327c380738ec61306c |
| packconv | hhhhhffd/conventor-microservice (published as conventor-mc-rp) | bc63a6373673552f9ee93be4efde6d6b5dae998e |
