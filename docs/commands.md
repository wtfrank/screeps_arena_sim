# Screeps Arena Simulator: Command Reference

The `screeps_arena_sim` binary provides administrative options to manage your bot library, configure arena aliases, inspect known real arenas, and set up simulation runs.

---

## Global Options

- `-l`, `--library-dir <PATH>`
  - Specifies the directory path where the bot library database and binaries are stored.
  - **Default**: `$XDG_DATA_HOME/screeps_arena_sim` or `~/.local/share/screeps_arena_sim` (fallback `./bot_library`).

---

## Commands

### `arena`
Subcommands to view real arenas.

#### `arena list`
Lists all known real arenas from the Screeps Arena platform.
- **Output Columns**:
  - `Arena Name`: Display name of the arena (e.g. `Spawn Strike 3`).
  - `Folder Name`: Stable folder identifier (e.g. `season3-spawn_strike`).
  - `Advanced`: Whether this is an advanced variant (`true`/`false`).
  - `Arena ID`: Canonical platform ID string (e.g. `69cfe6fcece2ae9f75da12d1`).

---

### `alias`
Subcommands to manage short unique aliases for real arena IDs.

#### `alias list`
Lists all defined arena aliases.
- **Output Columns**:
  - `Alias`: Unique short alias string (e.g. `ssb`).
  - `Arena Name`: Human-readable name (if provided).
  - `Arena ID`: Canonical platform arena ID.

#### `alias set <ALIAS> <ARENA_ID> [-n <NAME>]`
Creates or updates a unique short alias for a real arena ID.
- **Arguments**:
  - `ALIAS`: Short unique alias identifier (e.g. `ssb`).
  - `ARENA_ID`: Canonical arena ID string (e.g. `69cfe6fcece2ae9f75da12d1`).
  - `-n`, `--name`: Optional human-readable arena name (e.g. `"Spawn Strike 3"`).

#### `alias remove <ALIAS>`
Removes an existing arena alias.
- **Arguments**:
  - `ALIAS`: Short alias string to remove.

---

### `layout`
Subcommands to manage aliases for arena layout files or targets.

#### `layout list`
Lists all layout files stored in the XDG layout directory (`<library_dir>/layouts/`).
- **Output Columns**:
  - `Layout Alias`: Short layout alias (if configured).
  - `Game ID`: Platform game instance ID string.
  - `Arena ID`: Canonical arena ID string.
  - `Arena Name`: Human-readable arena name (e.g., `Spawn Strike 3`).

#### `layout set <ALIAS> <LAYOUT>`
Creates or updates a short alias mapping to a layout file or target.
- **Arguments**:
  - `ALIAS`: Short unique layout alias identifier (e.g. `layout1`).
  - `LAYOUT`: Target layout filename or path (e.g. `69cfe6fcece2ae9f75da12d1.json`).

#### `layout remove <ALIAS>`
Removes an existing layout alias.
- **Arguments**:
  - `ALIAS`: Short layout alias string to remove.

---

### `lib`
Subcommands to manage compiled bot binaries in the local library.

#### `lib list`
Lists all bot revisions currently stored in the library.
- **Output Columns**:
  - `ID`: Auto-incremented unique integer identifier.
  - `Visible Name`: Format `<name>:<version>` (e.g., `wtfbot:0`).
  - `Arena Link`: Real arena ID string linked to this bot.
  - `Binary Path`: Local storage path of the `.so` or `.dll` file.

#### `lib add <NAME> <ARENA> <PATH>`
Adds a new compiled bot binary to the library linked to a real arena ID or alias.
- **De-duplication**: Calculates the SHA256 checksum of the source binary. If the same binary already exists in the library, the addition is rejected.
- **Versioning**: Auto-assigns the next version revision (starting from `0`) for the bot name.
- **Arguments**:
  - `NAME`: Visible name family for the bot (e.g., `wtfbot`).
  - `ARENA`: Real arena ID or configured short alias.
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

---

### `run`
Runs a simulation match between two bots on a specified arena.

#### `run <BOT1> <BOT2> <ARENA> [-l <LAYOUT>] [-t <TICKS>]`
Executes a match loading an arena terrain layout.
- **Arguments**:
  - `BOT1`: Name:version or integer ID of Bot 1 (e.g. `wtfbot:0` or `1`).
  - `BOT2`: Name:version or integer ID of Bot 2 (e.g. `wtfbot:1` or `2`).
  - `ARENA`: Real arena ID or configured short alias.
  - `-l`, `--layout`: Optional specific layout game ID or layout alias (overrides random layout selection).
  - `-t`, `--ticks`: Maximum ticks to simulate (default: 1000).

