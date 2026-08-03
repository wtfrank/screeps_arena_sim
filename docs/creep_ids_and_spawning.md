# Creep IDs and Spawning Lifecycle in `screeps_arena_sim`

## Overview
In Screeps Arena, creep objects maintain a single, immutable, numeric string `id` (e.g. `"335"`) throughout their entire lifecycle—from the moment `spawnCreep()` is called until the creep is destroyed.

---

## 1. ID Pre-Allocation Pattern
To ensure client bots and the simulator engine agree on object IDs across IPC boundaries without round-trips:

- Each `StructureSpawn` holds a `next_id` string property (e.g. `"335"`).
- `next_id` is initialized at match startup to `max(layout_object_ids) + 1` and incremented monotonically for each spawn.
- When `StructureSpawn::spawn_creep(body)` is invoked by a bot, the returned `Creep` instance is immediately assigned `id = spawn.next_id`.

---

## 2. Spawning Lifecycle States

When a spawn starts spawning a creep:

1. **Tick 0 (`spawn_creep` called)**:
   - **`StructureSpawn`**: Sets `spawning = Some(Spawning { need_time, remaining_time })`.
   - **`Creep`**: Instantiated at the spawn position with `spawning = true` and `id = spawn.next_id`.
   - **Engine State**: Spawn's `next_id` is advanced to the next monotonic integer string (e.g. `"336"`).

2. **Mid-Spawn Ticks (`remaining_time > 0`)**:
   - `structure_spawn.spawning()` returns `Some(Spawning)`.
   - `creep.spawning()` returns `true`.
   - `creep.id()` remains fixed (`"335"`).
   - `game::utils::get_objects_by_prototype(CREEP)` includes the spawning creep.

3. **Spawn Completion (`remaining_time == 0`)**:
   - `structure_spawn.spawning` resets to `None`.
   - `creep.spawning` updates to `false`.
   - The creep keeps its exact same `id` (`"335"`) and becomes active for movement and actions.

---

## 3. Guarantees

- **Stable Object References**: References to `Creep` returned by `spawn_creep()` preserve their `.id` when stored across ticks (e.g., in `mem.spawning_roles` or `mem.roles`).
- **No ID Collisions**: IDs are monotonically assigned integers (`"335"`, `"336"`, `"337"`, ...) regardless of object deaths or parallel bot executions.
