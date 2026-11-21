#!/usr/bin/env bash
# Bakes blackhole's Boat snapshot: the pinned Rust and a warm target/.
set -euo pipefail
rustup toolchain install --no-self-update  # reads rust-toolchain.toml; bin/check refuses any other rustc
# Every git call passes --no-lazy-fetch, which needs git 2.45 or newer; Ubuntu 24.04 ships 2.43.
sudo add-apt-repository -y ppa:git-core/ppa >/dev/null
sudo apt-get install -y -qq git >/dev/null
have=$(git --version | awk '{print $3}')
[[ $(printf '%s\n' 2.45 "$have" | sort -V | head -1) == 2.45 ]] || { echo "git $have is older than 2.45" >&2; exit 1; }
cargo clippy --locked --all-targets
cargo test --locked --no-run
