# Bot Library
  1. List the current library:
    ./target/debug/screeps_arena_sim lib list

  2. Add a bot (will version dynamically starting at 0, copying and renaming the binary to bot_1.so):
    # Add wtfbot (first version will be wtfbot:0)
    ./target/debug/screeps_arena_sim lib add wtfbot spawn_strike_basic /path/to/compiled_bot.so

  3. Add another version of the same bot (will auto-increment to wtfbot:1, stored as bot_2.so):
    ./target/debug/screeps_arena_sim lib add wtfbot spawn_strike_basic /path/to/another_compiled_bot.so

  4. Rename the bot family (renames all revisions of wtfbot to my_bot):
    ./target/debug/screeps_arena_sim lib rename wtfbot my_bot

  5. Delete a specific version:
    ./target/debug/screeps_arena_sim lib delete my_bot:1

  6. Delete the entire family:
    ./target/debug/screeps_arena_sim lib delete my_bot


# Match Runner
1.  run a match
    cargo run -- run wtfbot:0 wtfbot:0 spawn_strike_basic
