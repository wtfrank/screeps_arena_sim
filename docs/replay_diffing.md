extract a replay from the capture

cargo run run ssb wtfbot:latest idle-bot:latest -r test-replay.json
cargo run diff test-replay.json ../screeps_arena_protocol/ticks.json
