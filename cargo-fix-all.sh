#!/bin/bash

# cargo fix -p display --target x86_64-unknown-linux-gnu --allow-dirty
cargo clippy --fix -p display --target x86_64-unknown-linux-gnu --allow-dirty

# cargo fix -p app --allow-dirty
cargo clippy --fix -p app  --allow-dirty

cargo fmt