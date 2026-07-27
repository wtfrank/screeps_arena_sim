# Screeps Arena Simulator: Command Reference

The `screeps_arena_sim` binary provides administrative options to manage your bot library and set up simulation runs.

---

## Global Options

- `-l`, `--library-dir <PATH>`
  - Specifies the directory path where the bot library database and binaries are stored.
  - **Default**: `./bot_library`

---

## Commands

### `lib`
Subcommands to manage the compiled bot binaries in the local library.

#### `lib list`
Lists all bot revisions currently stored in the library.
- **Output Columns**:
  - `ID`: Auto-incremented unique integer identifier.
  - `Visible Name`: Format `<name>:<version>` (e.g., `wtfbot:0`).
  - `Map Association`: The arena map this bot revision is built for.
  - `Binary Path`: Local storage path of the `.so` or `.dll` file.

#### `lib add <NAME> <MAP> <PATH>`
Adds a new compiled bot binary to the library under the specified name and arena association.
- **De-duplication**: Calculates the SHA256 checksum of the source binary. If the same binary already exists in the library, the addition is rejected.
- **Versioning**: Auto-assigns the next version revision (starting from `0`) for the bot name.
- **Arguments**:
  - `NAME`: Visible name family for the bot (e.g., `wtfbot`).
  - `MAP`: Associated map name. Standardized to full name when using shorthand aliases:
    - `spawn_strike_basic` (alias: `ssb`)
    - `spawn_strike_advanced` (alias: `ssa`)
    - `power_split_basic` (alias: `psb`)
    - `power_split_advanced` (alias: `psa`)
    - `escort_run_basic` (alias: `erb`)
    - `escort_run_advanced` (alias: `era`)
  - `PATH`: Path to the source compiled shared library.

#### `lib rename <OLD_NAME> <NEW_NAME>`
Renames a bot family across all of its versions.
- **Arguments**:
  - `OLD_NAME`: Current name of the family.
  - `NEW_NAME`: New name to assign.

#### `lib delete <NAME_OR_VERSION>`
Deletes bot revisions from the library and cleans up their local binary files.
- **Arguments**:
  - `NAME_OR_VERSION`: Can be a full visible name (e.g., `wtfbot:2`) to delete a specific revision, or just the family name (e.g., `wtfbot`) to delete all revisions of that bot.
