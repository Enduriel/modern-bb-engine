#!/bin/sh
# Take our frame goldens again (bb-app/tests/frame_regression.rs), after a change to what
# the game draws that is meant to show. They live outside the repository, in the folder
# BB_FRAME_GOLDENS names. Look at
# the new PNGs before relying on them.
set -e
cd "$(dirname "$0")/../.."
BB_FRAME_GOLDENS_UPDATE=1 cargo test --release -p bb-app --features ui --test frame_regression
