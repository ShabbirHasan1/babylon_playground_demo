#!/bin/sh

set -e
cargo build --release
scp target/release/babylon root@babylon:~/babylon
