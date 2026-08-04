use anyhow::{Context, Result};
use std::collections::HashMap;
use std::time::Duration;

use crate::driver::{BotDriver, QueuedAction};
use crate::models::{GameObject, GameState, Owner, Position, Ruleset, Terrain, WinCondition};
use screeps_arena::ffi::ActionId;

pub struct RunExecutor {
    state: GameState,
    bot1_driver: Option<BotDriver>,
    bot2_driver: Option<BotDriver>,
    rules: Ruleset,
    bot1_crashed: bool,
    bot2_crashed: bool,
    debug_b1: bool,
    debug_b2: bool,
    next_id: u32,
}

impl RunExecutor {
    pub fn new(
        mut initial_state: GameState,
        bot1_path: &std::path::Path,
        bot2_path: Option<&std::path::Path>,
        rules: Ruleset,
        debug_bot: Option<&str>,
    ) -> Result<Self> {
        let debug_b1 = match debug_bot {
            Some("1") | Some("bot1") | Some("all") => true,
            _ => false,
        };
        let debug_b2 = match debug_bot {
            Some("2") | Some("bot2") | Some("all") => true,
            _ => false,
        };

        let max_layout_id = initial_state
            .objects
            .iter()
            .filter_map(|o| match o {
                GameObject::Creep { id, .. }
                | GameObject::Spawn { id, .. }
                | GameObject::Tower { id, .. }
                | GameObject::Extension { id, .. }
                | GameObject::Rampart { id, .. }
                | GameObject::Container { id, .. }
                | GameObject::Road { id, .. }
                | GameObject::Wall { id, .. }
                | GameObject::Resource { id, .. }
                | GameObject::Source { id, .. }
                | GameObject::ConstructionSite { id, .. } => id.parse::<u32>().ok(),
                _ => None,
            })
            .max()
            .unwrap_or(0);

        let mut next_id = max_layout_id + 1;

        // Assign initial next_id to each spawn, and set fixed starting energy for spawns and extensions.
        for obj in &mut initial_state.objects {
            if let GameObject::Spawn { next_id: nid, energy, max_energy, .. } = obj {
                *nid = next_id.to_string();
                next_id += 1;
                *energy = 500;
                *max_energy = 1000;
            }
            if let GameObject::Extension { energy, max_energy, .. } = obj {
                *energy = 100;
                *max_energy = 100;
            }
        }

        let (bot1_driver, bot1_crashed) = match BotDriver::load(bot1_path, "Bot 1", debug_b1) {
            Ok(d) => (Some(d), false),
            Err(e) => {
                println!("Bot 1 crashed during initialization: {:?}", e);
                (None, true)
            }
        };

        let (bot2_driver, bot2_crashed) = if let Some(p2) = bot2_path {
            match BotDriver::load(p2, "Bot 2", debug_b2) {
                Ok(d) => (Some(d), false),
                Err(e) => {
                    println!("Bot 2 crashed during initialization: {:?}", e);
                    (None, true)
                }
            }
        } else {
            (None, false)
        };

        Ok(Self {
            state: initial_state,
            bot1_driver,
            bot2_driver,
            rules,
            bot1_crashed,
            bot2_crashed,
            debug_b1,
            debug_b2,
            next_id,
        })
    }

    pub fn bot1_crashed(&self) -> bool {
        self.bot1_crashed
    }

    pub fn bot2_crashed(&self) -> bool {
        self.bot2_crashed
    }

    /// Ticks the simulation once, returning true if the run has ended (win, loss, or draw).
    pub fn step_tick(&mut self) -> Result<Option<SimulationResult>> {
        if self.bot1_crashed && self.bot2_crashed {
            return Ok(Some(self.check_win_condition(false)));
        }

        if self.state.tick >= self.rules.tick_limit {
            return Ok(Some(self.check_win_condition(true)));
        }

        // 1. Prepare bot states
        let bot1_creeps = self.get_mock_creeps(true);
        let bot1_spawns = self.get_mock_spawns(true);
        let bot1_towers = self.get_mock_towers(true);
        let bot1_extensions = self.get_mock_extensions(true);
        let bot1_ramparts = self.get_mock_ramparts(true);
        let bot1_containers = self.get_mock_containers();
        let bot1_roads = self.get_mock_roads();
        let bot1_walls = self.get_mock_walls();
        let bot1_resources = self.get_mock_resources();
        let bot1_sources = self.get_mock_sources();
        let bot1_flags = self.get_mock_flags(true);
        let bot1_score_collectors = self.get_mock_score_collectors(true);
        let bot1_bonus_flags = self.get_mock_bonus_flags(true);
        let bot1_area_effects = self.get_mock_area_effects();
        let bot1_construction_sites = self.get_mock_construction_sites(true);
        let bot1_owned_structures = self.get_mock_owned_structures(true);

        let bot2_creeps = self.get_mock_creeps(false);
        let bot2_spawns = self.get_mock_spawns(false);
        let bot2_towers = self.get_mock_towers(false);
        let bot2_extensions = self.get_mock_extensions(false);
        let bot2_ramparts = self.get_mock_ramparts(false);
        let bot2_containers = self.get_mock_containers();
        let bot2_roads = self.get_mock_roads();
        let bot2_walls = self.get_mock_walls();
        let bot2_resources = self.get_mock_resources();
        let bot2_sources = self.get_mock_sources();
        let bot2_flags = self.get_mock_flags(false);
        let bot2_score_collectors = self.get_mock_score_collectors(false);
        let bot2_bonus_flags = self.get_mock_bonus_flags(false);
        let bot2_area_effects = self.get_mock_area_effects();
        let bot2_construction_sites = self.get_mock_construction_sites(false);
        let bot2_owned_structures = self.get_mock_owned_structures(false);

        // 2. Run active (non-crashed) bots in parallel threads
        let timeout_b1 = if self.debug_b1 {
            Duration::from_secs(3600)
        } else {
            Duration::from_millis(self.rules.cpu_time_limit as u64)
        };
        let timeout_b2 = if self.debug_b2 {
            Duration::from_secs(3600)
        } else {
            Duration::from_millis(self.rules.cpu_time_limit as u64)
        };
        let tick = self.state.tick;

        let (res1, res2) = thread_run_parallel(
            || {
                if !self.bot1_crashed {
                    if let Some(ref mut d1) = self.bot1_driver {
                        let msg = crate::driver::BotTickMessage {
                            tick,
                            is_bot_1: true,
                            creeps: bot1_creeps,
                            spawns: bot1_spawns,
                            towers: bot1_towers,
                            extensions: bot1_extensions,
                            ramparts: bot1_ramparts,
                            containers: bot1_containers,
                            roads: bot1_roads,
                            walls: bot1_walls,
                            resources: bot1_resources,
                            sources: bot1_sources,
                            flags: bot1_flags,
                            score_collectors: bot1_score_collectors,
                            bonus_flags: bot1_bonus_flags,
                            area_effects: bot1_area_effects,
                            construction_sites: bot1_construction_sites,
                            owned_structures: bot1_owned_structures,
                        };
                        d1.tick(msg, timeout_b1)
                    } else {
                        Ok(Vec::new())
                    }
                } else {
                    Ok(Vec::new())
                }
            },
            || {
                if !self.bot2_crashed {
                    if let Some(ref mut d2) = self.bot2_driver {
                        let msg = crate::driver::BotTickMessage {
                            tick,
                            is_bot_1: false,
                            creeps: bot2_creeps,
                            spawns: bot2_spawns,
                            towers: bot2_towers,
                            extensions: bot2_extensions,
                            ramparts: bot2_ramparts,
                            containers: bot2_containers,
                            roads: bot2_roads,
                            walls: bot2_walls,
                            resources: bot2_resources,
                            sources: bot2_sources,
                            flags: bot2_flags,
                            score_collectors: bot2_score_collectors,
                            bonus_flags: bot2_bonus_flags,
                            area_effects: bot2_area_effects,
                            construction_sites: bot2_construction_sites,
                            owned_structures: bot2_owned_structures,
                        };
                        d2.tick(msg, timeout_b2)
                    } else {
                        Ok(Vec::new())
                    }
                } else {
                    Ok(Vec::new())
                }
            },
        );

        let actions1 = match res1 {
            Ok(acts) => acts,
            Err(e) => {
                if !self.bot1_crashed {
                    println!("Bot 1 crashed: {:?}", e);
                    self.bot1_crashed = true;
                }
                Vec::new()
            }
        };

        let actions2 = match res2 {
            Ok(acts) => acts,
            Err(e) => {
                if !self.bot2_crashed {
                    println!("Bot 2 crashed: {:?}", e);
                    self.bot2_crashed = true;
                }
                Vec::new()
            }
        };

        // If both bots crashed, check win condition immediately or return draw
        if self.bot1_crashed && self.bot2_crashed {
            return Ok(Some(self.check_win_condition(false)));
        }

        // 3. Resolve actions
        self.resolve_actions(actions1, actions2);

        // 4. Update general state (ticks, fatigue recovery, etc.)
        self.state.tick += 1;
        self.apply_tick_decay();

        // 5. Check win conditions
        if let Some(res) = self.check_win_conditions_active() {
            return Ok(Some(res));
        }

        Ok(None)
    }

    fn check_win_condition(&self, limit_reached: bool) -> SimulationResult {
        let bot1_spawn_exists = self.state.objects.iter().any(|o| match o {
            GameObject::Spawn {
                owner: Owner::Bot1, ..
            } => true,
            _ => false,
        });
        let bot2_spawn_exists = self.state.objects.iter().any(|o| match o {
            GameObject::Spawn {
                owner: Owner::Bot2, ..
            } => true,
            _ => false,
        });

        match self.rules.win_condition {
            WinCondition::DestroyEnemySpawn => {
                if bot1_spawn_exists && !bot2_spawn_exists {
                    SimulationResult::Bot1Win {
                        reason: "Enemy spawn destroyed".to_string(),
                    }
                } else if !bot1_spawn_exists && bot2_spawn_exists {
                    SimulationResult::Bot2Win {
                        reason: "Spawn destroyed".to_string(),
                    }
                } else if limit_reached {
                    SimulationResult::Draw {
                        reason: "Tick limit reached".to_string(),
                    }
                } else if self.bot1_crashed && self.bot2_crashed {
                    SimulationResult::Draw {
                        reason: "Both bots crashed".to_string(),
                    }
                } else {
                    SimulationResult::Draw {
                        reason: "Both spawns destroyed simultaneously".to_string(),
                    }
                }
            }
            WinCondition::Survival => {
                if limit_reached {
                    SimulationResult::Draw {
                        reason: "Tick limit reached".to_string(),
                    }
                } else if !bot1_spawn_exists && !bot2_spawn_exists {
                    SimulationResult::Draw {
                        reason: "No survivors".to_string(),
                    }
                } else if bot1_spawn_exists {
                    SimulationResult::Bot1Win {
                        reason: "Survived".to_string(),
                    }
                } else {
                    SimulationResult::Bot2Win {
                        reason: "Survived".to_string(),
                    }
                }
            }
            WinCondition::HighestScore => SimulationResult::Draw {
                reason: "Score conditions unresolved".to_string(),
            },
        }
    }

    fn check_win_conditions_active(&self) -> Option<SimulationResult> {
        let bot1_spawn_exists = self.state.objects.iter().any(|o| match o {
            GameObject::Spawn {
                owner: Owner::Bot1, ..
            } => true,
            _ => false,
        });
        let bot2_spawn_exists = self.state.objects.iter().any(|o| match o {
            GameObject::Spawn {
                owner: Owner::Bot2, ..
            } => true,
            _ => false,
        });

        if !bot1_spawn_exists || !bot2_spawn_exists {
            return Some(self.check_win_condition(false));
        }
        None
    }

    fn resolve_actions(&mut self, actions1: Vec<QueuedAction>, actions2: Vec<QueuedAction>) {
        for (bot, actions) in [("Bot1", &actions1), ("Bot2", &actions2)] {
            for act in actions {
                log::debug!(
                    "[Tick {}] [{}] Action: {:?}, Actor: {}, Target: {:?}, Arg1: {}, Arg2: {}",
                    self.state.tick,
                    bot,
                    act.action,
                    act.actor_id,
                    act.target_id,
                    act.arg1,
                    act.arg2
                );
            }
        }
        // Resolve movement intents for non-fatigued creeps
        let mut move_intents: HashMap<String, Position> = HashMap::new();
        let mut current_positions: HashMap<String, Position> = HashMap::new();

        for obj in &self.state.objects {
            if let GameObject::Creep {
                id,
                pos,
                fatigue,
                spawning,
                ..
            } = obj
            {
                if *fatigue == 0 && !*spawning {
                    current_positions.insert(id.clone(), *pos);
                }
            }
        }

        for act in actions1
            .iter()
            .chain(actions2.iter())
            .filter(|a| a.action == ActionId::Move)
        {
            let id = &act.actor_id;
            if let Some(&curr_pos) = current_positions.get(id) {
                let dir = act.arg1 as u8;
                let mut target_pos = curr_pos;
                match dir {
                    1 => {
                        target_pos.y = target_pos.y.saturating_sub(1);
                    } // Top
                    2 => {
                        target_pos.x = target_pos.x.saturating_add(1);
                        target_pos.y = target_pos.y.saturating_sub(1);
                    } // TopRight
                    3 => {
                        target_pos.x = target_pos.x.saturating_add(1);
                    } // Right
                    4 => {
                        target_pos.x = target_pos.x.saturating_add(1);
                        target_pos.y = target_pos.y.saturating_add(1);
                    } // BottomRight
                    5 => {
                        target_pos.y = target_pos.y.saturating_add(1);
                    } // Bottom
                    6 => {
                        target_pos.x = target_pos.x.saturating_sub(1);
                        target_pos.y = target_pos.y.saturating_add(1);
                    } // BottomLeft
                    7 => {
                        target_pos.x = target_pos.x.saturating_sub(1);
                    } // Left
                    8 => {
                        target_pos.x = target_pos.x.saturating_sub(1);
                        target_pos.y = target_pos.y.saturating_sub(1);
                    } // TopLeft
                    _ => {}
                }

                // Verify inside arena bounds and not moving into a Wall
                if target_pos.x < self.state.width && target_pos.y < self.state.height {
                    let terrain = self.state.terrain[target_pos.x as usize][target_pos.y as usize];
                    if terrain != Terrain::Wall {
                        move_intents.insert(id.clone(), target_pos);
                    }
                }
            }
        }

        // Collect static obstacles: non-moving creeps, spawns, towers, extensions, ramparts, constructed walls
        let mut blocked_tiles: std::collections::HashSet<Position> =
            std::collections::HashSet::new();

        for obj in &self.state.objects {
            match obj {
                GameObject::Creep { id, pos, .. } => {
                    if !move_intents.contains_key(id) {
                        blocked_tiles.insert(*pos);
                    }
                }
                GameObject::Spawn { pos, .. }
                | GameObject::Tower { pos, .. }
                | GameObject::Extension { pos, .. }
                | GameObject::Wall { pos, .. } => {
                    blocked_tiles.insert(*pos);
                }
                _ => {}
            }
        }

        // Reject move intents if multiple creeps target the exact same tile
        let mut target_counts: HashMap<Position, usize> = HashMap::new();
        for target in move_intents.values() {
            *target_counts.entry(*target).or_insert(0) += 1;
        }

        // Iteratively resolve valid moves (allowing chaining & swapping)
        let mut approved_moves: HashMap<String, Position> = HashMap::new();
        let mut resolved = true;

        while resolved {
            resolved = false;
            let current_intents: Vec<(String, Position)> = move_intents
                .iter()
                .filter(|(id, _)| !approved_moves.contains_key(*id))
                .map(|(id, p)| (id.clone(), *p))
                .collect();

            for (creep_id, target_pos) in current_intents {
                // Reject if multiple creeps contend for the same destination tile
                if target_counts.get(&target_pos).cloned().unwrap_or(0) > 1 {
                    move_intents.remove(&creep_id);
                    if let Some(&start) = current_positions.get(&creep_id) {
                        blocked_tiles.insert(start);
                    }
                    resolved = true;
                    continue;
                }

                // Check if target position is blocked by a static obstacle
                if blocked_tiles.contains(&target_pos) {
                    move_intents.remove(&creep_id);
                    if let Some(&start) = current_positions.get(&creep_id) {
                        blocked_tiles.insert(start);
                    }
                    resolved = true;
                    continue;
                }

                // Check if target position is occupied by another moving creep
                let occupant = current_positions
                    .iter()
                    .find(|&(ref other_id, &pos)| pos == target_pos && **other_id != creep_id)
                    .map(|(other_id, _)| other_id.clone());

                match occupant {
                    None => {
                        // Target tile is vacant -> move approved!
                        approved_moves.insert(creep_id, target_pos);
                        resolved = true;
                    }
                    Some(other_id) => {
                        if let Some(&other_target) = move_intents.get(&other_id) {
                            let curr_pos = current_positions[&creep_id];
                            // Check for position swap (A -> B and B -> A)
                            let is_swap = other_target == curr_pos;

                            if is_swap {
                                // Swap approved!
                                approved_moves.insert(creep_id, target_pos);
                                approved_moves.insert(other_id, curr_pos);
                                resolved = true;
                            } else if approved_moves.contains_key(&other_id) {
                                // Occupant has already successfully moved out -> chain move approved!
                                approved_moves.insert(creep_id, target_pos);
                                resolved = true;
                            }
                        }
                    }
                }
            }
        }

        // Apply approved moves and calculate fatigue according to Screeps rules
        for obj in &mut self.state.objects {
            if let GameObject::Creep {
                id,
                pos,
                fatigue,
                body,
                store,
                ..
            } = obj
            {
                if let Some(&new_pos) = approved_moves.get(id) {
                    *pos = new_pos;
                    let move_parts = body
                        .iter()
                        .filter(|&&p| p == screeps_arena::constants::Part::Move)
                        .count() as u32;
                    let carry_parts = body
                        .iter()
                        .filter(|&&p| p == screeps_arena::constants::Part::Carry)
                        .count() as u32;
                    let store_used: u32 = store.values().sum();
                    let active_carries = (store_used.div_ceil(50)).min(carry_parts);

                    let non_carry_non_move_weight = body
                        .iter()
                        .filter(|&&p| {
                            p != screeps_arena::constants::Part::Move
                                && p != screeps_arena::constants::Part::Carry
                        })
                        .count() as u32;
                    let total_weight = non_carry_non_move_weight + active_carries;

                    let terrain = self.state.terrain[new_pos.x as usize][new_pos.y as usize];
                    let terrain_cost = if terrain == Terrain::Swamp { 10 } else { 2 };
                    let added_fatigue =
                        (total_weight * terrain_cost).saturating_sub(move_parts * 2);

                    *fatigue = (*fatigue as u32 + added_fatigue).min(255) as u8;
                }
            }
        }

        // Resolve attacks (towers & creeps)
        let mut damage_map = HashMap::new();
        for act in actions1.iter().chain(actions2.iter()) {
            if act.action == ActionId::Attack {
                if let Some(ref target) = act.target_id {
                    *damage_map.entry(target.clone()).or_insert(0) += 30; // standard creep attack
                }
            }
        }

        // Apply damages
        for obj in &mut self.state.objects {
            match obj {
                GameObject::Creep { id, hits, .. } => {
                    if let Some(&dmg) = damage_map.get(id) {
                        *hits = hits.saturating_sub(dmg);
                    }
                }
                GameObject::Spawn { id, hits, .. } => {
                    if let Some(&dmg) = damage_map.get(id) {
                        *hits = hits.saturating_sub(dmg);
                    }
                }
                GameObject::Tower { id, hits, .. } => {
                    if let Some(&dmg) = damage_map.get(id) {
                        *hits = hits.saturating_sub(dmg);
                    }
                }
                _ => {}
            }
        }

        // Resolve SpawnCreep actions
        for (act, owner) in actions1
            .iter()
            .map(|a| (a, Owner::Bot1))
            .chain(actions2.iter().map(|a| (a, Owner::Bot2)))
        {
            if act.action == ActionId::SpawnCreep {
                let spawn_id = &act.actor_id;
                let body_len = act.arg1 as u32;

                // Calculate energy cost (assuming standard 100 energy per body part if default or decode from arg2)
                let energy_cost = if body_len > 0 { body_len * 100 } else { 200 };

                // Verify spawn ownership and presence, checking energy within SPAWN_RANGE
                let mut spawn_pos = None;
                let mut available_energy = 0;
                let range = screeps_arena::constants::SPAWN_RANGE as u8;

                for obj in &self.state.objects {
                    if let GameObject::Spawn {
                        id, owner: o, pos, spawning, ..
                    } = obj
                    {
                        if id == spawn_id && *o == owner && spawning.is_none() {
                            spawn_pos = Some(*pos);
                            break;
                        }
                    }
                }

                if let Some(spos) = spawn_pos {
                    for obj in &self.state.objects {
                        match obj {
                            GameObject::Spawn {
                                owner: o,
                                energy,
                                pos,
                                ..
                            } if *o == owner
                                && pos.x.abs_diff(spos.x) <= range
                                && pos.y.abs_diff(spos.y) <= range =>
                            {
                                available_energy += *energy;
                            }
                            GameObject::Extension {
                                owner: o,
                                energy,
                                pos,
                                ..
                            } if *o == owner
                                && pos.x.abs_diff(spos.x) <= range
                                && pos.y.abs_diff(spos.y) <= range =>
                            {
                                available_energy += *energy;
                            }
                            _ => {}
                        }
                    }

                    if available_energy >= energy_cost {
                        let mut remaining_needed = energy_cost;

                        // Deduct energy from extensions first, then spawns (within SPAWN_RANGE)
                        for obj in &mut self.state.objects {
                            if remaining_needed == 0 {
                                break;
                            }
                            match obj {
                                GameObject::Extension {
                                    owner: o,
                                    energy,
                                    pos,
                                    ..
                                } if *o == owner
                                    && *energy > 0
                                    && pos.x.abs_diff(spos.x) <= range
                                    && pos.y.abs_diff(spos.y) <= range =>
                                {
                                    let deduct = (*energy).min(remaining_needed);
                                    *energy -= deduct;
                                    remaining_needed -= deduct;
                                }
                                GameObject::Spawn {
                                    owner: o,
                                    energy,
                                    pos,
                                    ..
                                } if *o == owner
                                    && *energy > 0
                                    && pos.x.abs_diff(spos.x) <= range
                                    && pos.y.abs_diff(spos.y) <= range =>
                                {
                                    let deduct = (*energy).min(remaining_needed);
                                    *energy -= deduct;
                                    remaining_needed -= deduct;
                                }
                                _ => {}
                            }
                        }

                        // Find spawn's next_id for this creep, then allocate a new next_id for the spawn
                        let mut assigned_creep_id = None;
                        let new_allocated_id = self.next_id.to_string();
                        self.next_id += 1;

                        for obj in &mut self.state.objects {
                            if let GameObject::Spawn {
                                id, next_id: nid, ..
                            } = obj
                            {
                                if id == spawn_id {
                                    assigned_creep_id = Some(nid.clone());
                                    *nid = new_allocated_id.clone();
                                    break;
                                }
                            }
                        }

                        let new_creep_id = assigned_creep_id.unwrap_or_else(|| {
                            let id = self.next_id.to_string();
                            self.next_id += 1;
                            id
                        });

                        // Create the new creep with spawning = true (takes 3 ticks per body part, e.g. body_len * 3)
                        let need_time = (body_len.max(1) * 3);

                        let spawn_progress = crate::models::SpawningProgress {
                            creep_id: new_creep_id.clone(),
                            need_time,
                            remaining_time: need_time,
                        };

                        // Set spawning state on the spawn structure
                        for obj in &mut self.state.objects {
                            if let GameObject::Spawn { id, spawning, .. } = obj {
                                if id == spawn_id {
                                    *spawning = Some(spawn_progress.clone());
                                }
                            }
                        }

                        // Add new creep with spawning: true
                        self.state.objects.push(GameObject::Creep {
                            id: new_creep_id,
                            pos: spos,
                            hits: (body_len.max(1) * 100),
                            max_hits: (body_len.max(1) * 100),
                            owner,
                            fatigue: 0,
                            spawning: true,
                            body: Vec::new(),
                            store: HashMap::new(),
                        });
                    }
                }
            }
        }

        // Remove destroyed units
        self.state.objects.retain(|o| match o {
            GameObject::Creep { hits, .. } => *hits > 0,
            GameObject::Spawn { hits, .. } => *hits > 0,
            GameObject::Tower { hits, .. } => *hits > 0,
            _ => true,
        });
    }

    fn apply_tick_decay(&mut self) {
        let mut completed_creep_ids = Vec::new();

        for obj in &mut self.state.objects {
            if let GameObject::Creep { fatigue, body, .. } = obj {
                let move_parts = body
                    .iter()
                    .filter(|&&p| p == screeps_arena::constants::Part::Move)
                    .count() as u8;
                let decay = if move_parts > 0 { move_parts * 2 } else { 2 };
                *fatigue = fatigue.saturating_sub(decay);
            }
            if let GameObject::Spawn { pos, spawning, .. } = obj {
                if let Some(progress) = spawning {
                    progress.remaining_time = progress.remaining_time.saturating_sub(1);
                    if progress.remaining_time == 0 {
                        completed_creep_ids.push((*pos, progress.creep_id.clone()));
                        *spawning = None;
                    }
                }
            }
        }
        for (spawn_pos, creep_id) in completed_creep_ids {
            let mut free_spot = None;

            // Search 8 adjacent positions around spawn_pos
            for dx in [-1i32, 0, 1] {
                for dy in [-1i32, 0, 1] {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let nx = spawn_pos.x as i32 + dx;
                    let ny = spawn_pos.y as i32 + dy;

                    if nx >= 0 && nx < self.state.width as i32 && ny >= 0 && ny < self.state.height as i32 {
                        let candidate = Position { x: nx as u8, y: ny as u8 };
                        if self.state.terrain[candidate.x as usize][candidate.y as usize] == Terrain::Wall {
                            continue;
                        }

                        // Check if tile is occupied by static structure or another creep
                        let occupied = self.state.objects.iter().any(|o| match o {
                            GameObject::Creep { pos, .. } => *pos == candidate,
                            GameObject::Spawn { pos, .. }
                            | GameObject::Tower { pos, .. }
                            | GameObject::Extension { pos, .. }
                            | GameObject::Wall { pos, .. } => *pos == candidate,
                            _ => false,
                        });

                        if !occupied {
                            free_spot = Some(candidate);
                            break;
                        }
                    }
                }
                if free_spot.is_some() {
                    break;
                }
            }

            if let Some(new_pos) = free_spot {
                for obj in &mut self.state.objects {
                    if let GameObject::Creep { id, pos, spawning, .. } = obj {
                        if id == &creep_id {
                            *pos = new_pos;
                            *spawning = false;
                        }
                    }
                }
            } else {
                // No free adjacent spot available; re-mark spawn as occupied so it retries next tick
                for obj in &mut self.state.objects {
                    if let GameObject::Spawn { pos, spawning, .. } = obj {
                        if *pos == spawn_pos {
                            *spawning = Some(crate::models::SpawningProgress {
                                creep_id: creep_id.clone(),
                                need_time: 1,
                                remaining_time: 1,
                            });
                        }
                    }
                }
            }
        }
        // Regenerate 1 energy per tick on each spawn, up to max_energy.
        for obj in &mut self.state.objects {
            if let GameObject::Spawn { energy, max_energy, .. } = obj {
                *energy = (*energy + 1).min(*max_energy);
            }
        }
    }

    // Helper functions to construct mock_screeps_arena structures with proper `my()` logic:

    fn get_mock_creeps(&self, is_bot1: bool) -> Vec<screeps_arena::objects::Creep> {
        self.state
            .objects
            .iter()
            .filter_map(|o| match o {
                GameObject::Creep {
                    id,
                    pos,
                    hits,
                    max_hits,
                    owner,
                    fatigue,
                    spawning,
                    ..
                } => {
                    let my = if is_bot1 {
                        *owner == Owner::Bot1
                    } else {
                        *owner == Owner::Bot2
                    };
                    Some(screeps_arena::objects::Creep {
                        base: screeps_arena::objects::GameObject {
                            id: id.clone(),
                            x: pos.x,
                            y: pos.y,
                        },
                        hits: *hits,
                        hits_max: *max_hits,
                        fatigue: *fatigue as u32,
                        my,
                        spawning: *spawning,
                    })
                }
                _ => None,
            })
            .collect()
    }

    fn get_mock_spawns(&self, is_bot1: bool) -> Vec<screeps_arena::objects::StructureSpawn> {
        self.state
            .objects
            .iter()
            .filter_map(|o| match o {
                GameObject::Spawn {
                    id,
                    pos,
                    hits,
                    max_hits,
                    owner,
                    energy,
                    max_energy,
                    spawning,
                    next_id,
                } => {
                    let my = if is_bot1 {
                        *owner == Owner::Bot1
                    } else {
                        *owner == Owner::Bot2
                    };
                    let mock_spawning =
                        spawning.as_ref().map(|s| screeps_arena::objects::Spawning {
                            need_time: s.need_time,
                            remaining_time: s.remaining_time,
                        });
                    Some(screeps_arena::objects::StructureSpawn {
                        base: screeps_arena::objects::GameObject {
                            id: id.clone(),
                            x: pos.x,
                            y: pos.y,
                        },
                        hits: *hits,
                        hits_max: *max_hits,
                        energy: *energy,
                        energy_max: *max_energy,
                        my: Some(my),
                        spawning: mock_spawning,
                        next_id: next_id.clone(),
                    })
                }
                _ => None,
            })
            .collect()
    }

    fn get_mock_towers(&self, is_bot1: bool) -> Vec<screeps_arena::objects::StructureTower> {
        self.state
            .objects
            .iter()
            .filter_map(|o| match o {
                GameObject::Tower {
                    id,
                    pos,
                    hits,
                    max_hits,
                    owner,
                    energy,
                    max_energy,
                } => {
                    let my = if is_bot1 {
                        *owner == Owner::Bot1
                    } else {
                        *owner == Owner::Bot2
                    };
                    Some(screeps_arena::objects::StructureTower {
                        base: screeps_arena::objects::GameObject {
                            id: id.clone(),
                            x: pos.x,
                            y: pos.y,
                        },
                        hits: *hits,
                        hits_max: *max_hits,
                        energy: *energy,
                        energy_max: *max_energy,
                        my: Some(my),
                    })
                }
                _ => None,
            })
            .collect()
    }

    fn get_mock_extensions(
        &self,
        is_bot1: bool,
    ) -> Vec<screeps_arena::objects::StructureExtension> {
        self.state
            .objects
            .iter()
            .filter_map(|o| match o {
                GameObject::Extension {
                    id,
                    pos,
                    hits,
                    max_hits,
                    owner,
                    energy,
                    max_energy,
                } => {
                    let my = if is_bot1 {
                        *owner == Owner::Bot1
                    } else {
                        *owner == Owner::Bot2
                    };
                    Some(screeps_arena::objects::StructureExtension {
                        base: screeps_arena::objects::GameObject {
                            id: id.clone(),
                            x: pos.x,
                            y: pos.y,
                        },
                        hits: *hits,
                        hits_max: *max_hits,
                        energy: *energy,
                        energy_max: *max_energy,
                        my: Some(my),
                    })
                }
                _ => None,
            })
            .collect()
    }

    fn get_mock_ramparts(&self, is_bot1: bool) -> Vec<screeps_arena::objects::StructureRampart> {
        self.state
            .objects
            .iter()
            .filter_map(|o| match o {
                GameObject::Rampart {
                    id,
                    pos,
                    hits,
                    max_hits,
                    owner,
                } => {
                    let my = if is_bot1 {
                        *owner == Owner::Bot1
                    } else {
                        *owner == Owner::Bot2
                    };
                    Some(screeps_arena::objects::StructureRampart {
                        base: screeps_arena::objects::GameObject {
                            id: id.clone(),
                            x: pos.x,
                            y: pos.y,
                        },
                        hits: *hits,
                        hits_max: *max_hits,
                        my: Some(my),
                    })
                }
                _ => None,
            })
            .collect()
    }

    fn get_mock_containers(&self) -> Vec<screeps_arena::objects::StructureContainer> {
        self.state
            .objects
            .iter()
            .filter_map(|o| match o {
                GameObject::Container {
                    id,
                    pos,
                    hits,
                    max_hits,
                    energy,
                    max_energy,
                } => Some(screeps_arena::objects::StructureContainer {
                    base: screeps_arena::objects::GameObject {
                        id: id.clone(),
                        x: pos.x,
                        y: pos.y,
                    },
                    hits: *hits,
                    hits_max: *max_hits,
                    store: *energy,
                    store_max: *max_energy,
                }),
                _ => None,
            })
            .collect()
    }

    fn get_mock_roads(&self) -> Vec<screeps_arena::objects::StructureRoad> {
        self.state
            .objects
            .iter()
            .filter_map(|o| match o {
                GameObject::Road {
                    id,
                    pos,
                    hits,
                    max_hits,
                } => Some(screeps_arena::objects::StructureRoad {
                    base: screeps_arena::objects::GameObject {
                        id: id.clone(),
                        x: pos.x,
                        y: pos.y,
                    },
                    hits: *hits,
                    hits_max: *max_hits,
                }),
                _ => None,
            })
            .collect()
    }

    fn get_mock_walls(&self) -> Vec<screeps_arena::objects::StructureWall> {
        self.state
            .objects
            .iter()
            .filter_map(|o| match o {
                GameObject::Wall {
                    id,
                    pos,
                    hits,
                    max_hits,
                } => Some(screeps_arena::objects::StructureWall {
                    base: screeps_arena::objects::GameObject {
                        id: id.clone(),
                        x: pos.x,
                        y: pos.y,
                    },
                    hits: *hits,
                    hits_max: *max_hits,
                }),
                _ => None,
            })
            .collect()
    }

    fn get_mock_resources(&self) -> Vec<screeps_arena::objects::Resource> {
        self.state
            .objects
            .iter()
            .filter_map(|o| match o {
                GameObject::Resource {
                    id,
                    pos,
                    amount,
                    resource_type,
                } => Some(screeps_arena::objects::Resource {
                    base: screeps_arena::objects::GameObject {
                        id: id.clone(),
                        x: pos.x,
                        y: pos.y,
                    },
                    amount: *amount,
                    resource_type: resource_type.clone(),
                }),
                _ => None,
            })
            .collect()
    }

    fn get_mock_sources(&self) -> Vec<screeps_arena::objects::Source> {
        self.state
            .objects
            .iter()
            .filter_map(|o| match o {
                GameObject::Source {
                    id,
                    pos,
                    energy,
                    max_energy,
                } => Some(screeps_arena::objects::Source {
                    base: screeps_arena::objects::GameObject {
                        id: id.clone(),
                        x: pos.x,
                        y: pos.y,
                    },
                    energy: *energy,
                    energy_max: *max_energy,
                }),
                _ => None,
            })
            .collect()
    }

    fn get_mock_flags(&self, is_bot1: bool) -> Vec<screeps_arena::objects::Flag> {
        self.state
            .objects
            .iter()
            .filter_map(|o| match o {
                GameObject::Flag { id, pos, owner } => {
                    let my = if is_bot1 {
                        *owner == Owner::Bot1
                    } else {
                        *owner == Owner::Bot2
                    };
                    Some(screeps_arena::objects::Flag {
                        base: screeps_arena::objects::GameObject {
                            id: id.clone(),
                            x: pos.x,
                            y: pos.y,
                        },
                        my: Some(my),
                    })
                }
                _ => None,
            })
            .collect()
    }

    fn get_mock_score_collectors(
        &self,
        is_bot1: bool,
    ) -> Vec<screeps_arena::objects::ScoreCollector> {
        self.state
            .objects
            .iter()
            .filter_map(|o| match o {
                GameObject::ScoreCollector { id, pos, owner } => {
                    let my = if is_bot1 {
                        *owner == Owner::Bot1
                    } else {
                        *owner == Owner::Bot2
                    };
                    Some(screeps_arena::objects::ScoreCollector {
                        base: screeps_arena::objects::GameObject {
                            id: id.clone(),
                            x: pos.x,
                            y: pos.y,
                        },
                        my,
                    })
                }
                _ => None,
            })
            .collect()
    }

    fn get_mock_bonus_flags(&self, is_bot1: bool) -> Vec<screeps_arena::objects::BonusFlag> {
        self.state
            .objects
            .iter()
            .filter_map(|o| match o {
                GameObject::BonusFlag { id, pos, owner } => {
                    let my = if is_bot1 {
                        *owner == Owner::Bot1
                    } else {
                        *owner == Owner::Bot2
                    };
                    Some(screeps_arena::objects::BonusFlag {
                        base: screeps_arena::objects::GameObject {
                            id: id.clone(),
                            x: pos.x,
                            y: pos.y,
                        },
                        my: Some(my),
                    })
                }
                _ => None,
            })
            .collect()
    }

    fn get_mock_area_effects(&self) -> Vec<screeps_arena::objects::AreaEffect> {
        self.state
            .objects
            .iter()
            .filter_map(|o| match o {
                GameObject::AreaEffect {
                    id,
                    pos,
                    effect_type,
                } => Some(screeps_arena::objects::AreaEffect {
                    base: screeps_arena::objects::GameObject {
                        id: id.clone(),
                        x: pos.x,
                        y: pos.y,
                    },
                    effect_type: effect_type.clone(),
                }),
                _ => None,
            })
            .collect()
    }

    fn get_mock_construction_sites(
        &self,
        is_bot1: bool,
    ) -> Vec<screeps_arena::objects::ConstructionSite> {
        self.state
            .objects
            .iter()
            .filter_map(|o| match o {
                GameObject::ConstructionSite {
                    id,
                    pos,
                    owner,
                    progress,
                    progress_total,
                } => {
                    let my = if is_bot1 {
                        *owner == Owner::Bot1
                    } else {
                        *owner == Owner::Bot2
                    };
                    Some(screeps_arena::objects::ConstructionSite {
                        base: screeps_arena::objects::GameObject {
                            id: id.clone(),
                            x: pos.x,
                            y: pos.y,
                        },
                        my,
                        progress: *progress,
                        progress_total: *progress_total,
                    })
                }
                _ => None,
            })
            .collect()
    }

    fn get_mock_owned_structures(&self, is_bot1: bool) -> Vec<screeps_arena::objects::OwnedStructure> {
        let mut result = Vec::new();

        for obj in &self.state.objects {
            match obj {
                GameObject::Spawn { id, pos, owner, .. }
                | GameObject::Tower { id, pos, owner, .. }
                | GameObject::Extension { id, pos, owner, .. } => {
                    let my = if is_bot1 { *owner == Owner::Bot1 } else { *owner == Owner::Bot2 };
                    result.push(screeps_arena::objects::OwnedStructure {
                        base: screeps_arena::objects::Structure {
                            base: screeps_arena::objects::GameObject {
                                id: id.clone(),
                                x: pos.x,
                                y: pos.y,
                            },
                        },
                        my: Some(my),
                    });
                }
                _ => {}
            }
        }

        result
    }
}

#[derive(Debug, Clone)]
pub enum SimulationResult {
    Bot1Win { reason: String },
    Bot2Win { reason: String },
    Draw { reason: String },
}

/// Helper function to concurrently execute two bot calculations in parallel.
fn thread_run_parallel<F1, F2, T1, T2>(f1: F1, f2: F2) -> (Result<T1>, Result<T2>)
where
    F1: FnOnce() -> Result<T1> + Send,
    F2: FnOnce() -> Result<T2> + Send,
    T1: Send,
    T2: Send,
{
    std::thread::scope(|s| {
        let handle1 = s.spawn(|| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(f1))
                .unwrap_or_else(|_| Err(anyhow::anyhow!("Bot A thread panicked")))
        });
        let handle2 = s.spawn(|| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(f2))
                .unwrap_or_else(|_| Err(anyhow::anyhow!("Bot B thread panicked")))
        });

        let res1 = handle1
            .join()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("Bot A thread join failed")));
        let res2 = handle2
            .join()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("Bot B thread join failed")));

        (res1, res2)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::QueuedAction;
    use crate::models::{GameObject, GameState, Owner, Position, Ruleset, Terrain};
    use screeps_arena::ffi::ActionId;
    use std::collections::HashMap;

    fn create_test_executor() -> RunExecutor {
        let state = GameState {
            tick: 1,
            width: 100,
            height: 100,
            terrain: vec![vec![Terrain::Plain; 100]; 100],
            objects: Vec::new(),
        };
        RunExecutor {
            rules: Ruleset {
                tick_limit: 2000,
                cpu_time_limit: 50,
                win_condition: crate::models::WinCondition::DestroyEnemySpawn,
            },
            state,
            bot1_driver: None,
            bot2_driver: None,
            bot1_crashed: false,
            bot2_crashed: false,
            debug_b1: false,
            debug_b2: false,
            next_id: 1000,
        }
    }

    #[test]
    fn test_fatigue_decay() {
        let mut exec = create_test_executor();
        exec.state.objects.push(GameObject::Creep {
            id: "creep1".to_string(),
            pos: Position { x: 10, y: 10 },
            hits: 100,
            max_hits: 100,
            owner: Owner::Bot1,
            fatigue: 5,
            spawning: false,
            body: vec![screeps_arena::constants::Part::Move, screeps_arena::constants::Part::Move],
            store: HashMap::new(),
        });

        // 2 MOVE parts decay 4 fatigue per tick down to 0
        exec.apply_tick_decay();
        if let GameObject::Creep { fatigue, .. } = &exec.state.objects[0] {
            assert_eq!(*fatigue, 1);
        } else {
            panic!("Expected creep object");
        }

        exec.apply_tick_decay();
        if let GameObject::Creep { fatigue, .. } = &exec.state.objects[0] {
            assert_eq!(*fatigue, 0);
        } else {
            panic!("Expected creep object");
        }
    }

    #[test]
    fn test_movement_position_swap() {
        let mut exec = create_test_executor();
        // Creep 1 at (10, 10), Creep 2 at (11, 10)
        exec.state.objects.push(GameObject::Creep {
            id: "creep1".to_string(),
            pos: Position { x: 10, y: 10 },
            hits: 100,
            max_hits: 100,
            owner: Owner::Bot1,
            fatigue: 0,
            spawning: false,
            body: vec![screeps_arena::constants::Part::Move],
            store: HashMap::new(),
        });
        exec.state.objects.push(GameObject::Creep {
            id: "creep2".to_string(),
            pos: Position { x: 11, y: 10 },
            hits: 100,
            max_hits: 100,
            owner: Owner::Bot2,
            fatigue: 0,
            spawning: false,
            body: vec![screeps_arena::constants::Part::Move],
            store: HashMap::new(),
        });

        // Creep 1 moves Right (3 -> x+1, (11,10))
        // Creep 2 moves Left (7 -> x-1, (10,10))
        let actions1 = vec![QueuedAction {
            actor_id: "creep1".to_string(),
            action: ActionId::Move,
            target_id: None,
            arg1: 3,
            arg2: 0,
        }];
        let actions2 = vec![QueuedAction {
            actor_id: "creep2".to_string(),
            action: ActionId::Move,
            target_id: None,
            arg1: 7,
            arg2: 0,
        }];

        exec.resolve_actions(actions1, actions2);

        // Position swap must succeed
        if let GameObject::Creep { pos, .. } = &exec.state.objects[0] {
            assert_eq!(*pos, Position { x: 11, y: 10 });
        }
        if let GameObject::Creep { pos, .. } = &exec.state.objects[1] {
            assert_eq!(*pos, Position { x: 10, y: 10 });
        }
    }

    #[test]
    fn test_contested_tile_rejection() {
        let mut exec = create_test_executor();
        // Creep 1 at (10, 10), Creep 2 at (12, 10) targeting (11, 10)
        exec.state.objects.push(GameObject::Creep {
            id: "creep1".to_string(),
            pos: Position { x: 10, y: 10 },
            hits: 100,
            max_hits: 100,
            owner: Owner::Bot1,
            fatigue: 0,
            spawning: false,
            body: vec![screeps_arena::constants::Part::Move],
            store: HashMap::new(),
        });
        exec.state.objects.push(GameObject::Creep {
            id: "creep2".to_string(),
            pos: Position { x: 12, y: 10 },
            hits: 100,
            max_hits: 100,
            owner: Owner::Bot2,
            fatigue: 0,
            spawning: false,
            body: vec![screeps_arena::constants::Part::Move],
            store: HashMap::new(),
        });

        // Both move into (11, 10)
        let actions1 = vec![QueuedAction {
            actor_id: "creep1".to_string(),
            action: ActionId::Move,
            target_id: None,
            arg1: 3, // Right
            arg2: 0,
        }];
        let actions2 = vec![QueuedAction {
            actor_id: "creep2".to_string(),
            action: ActionId::Move,
            target_id: None,
            arg1: 7, // Left
            arg2: 0,
        }];

        exec.resolve_actions(actions1, actions2);

        // Both moves fail and stay put
        if let GameObject::Creep { pos, .. } = &exec.state.objects[0] {
            assert_eq!(*pos, Position { x: 10, y: 10 });
        }
        if let GameObject::Creep { pos, .. } = &exec.state.objects[1] {
            assert_eq!(*pos, Position { x: 12, y: 10 });
        }
    }

    #[test]
    fn test_move_into_wall_rejected() {
        let mut exec = create_test_executor();
        exec.state.terrain[11][10] = Terrain::Wall;
        exec.state.objects.push(GameObject::Creep {
            id: "creep1".to_string(),
            pos: Position { x: 10, y: 10 },
            hits: 100,
            max_hits: 100,
            owner: Owner::Bot1,
            fatigue: 0,
            spawning: false,
            body: vec![screeps_arena::constants::Part::Move],
            store: HashMap::new(),
        });

        let actions1 = vec![QueuedAction {
            actor_id: "creep1".to_string(),
            action: ActionId::Move,
            target_id: None,
            arg1: 3, // Right into Wall
            arg2: 0,
        }];

        exec.resolve_actions(actions1, Vec::new());

        if let GameObject::Creep { pos, .. } = &exec.state.objects[0] {
            assert_eq!(*pos, Position { x: 10, y: 10 });
        }
    }

    #[test]
    fn test_spawn_creep_energy_range_and_busy() {
        let mut exec = create_test_executor();

        // Friendly spawn at (10, 10) with 300 energy
        exec.state.objects.push(GameObject::Spawn {
            id: "spawn1".to_string(),
            pos: Position { x: 10, y: 10 },
            hits: 5000,
            max_hits: 5000,
            owner: Owner::Bot1,
            energy: 300,
            max_energy: 300,
            spawning: None,
            next_id: "creep101".to_string(),
        });

        // Spawn 1 creep with 2 body parts (cost 200)
        let actions1 = vec![QueuedAction {
            actor_id: "spawn1".to_string(),
            action: ActionId::SpawnCreep,
            target_id: None,
            arg1: 2,
            arg2: 0,
        }];

        exec.resolve_actions(actions1.clone(), Vec::new());

        // Spawn energy should be deducted (300 -> 100) and spawning state set
        if let GameObject::Spawn { energy, spawning, .. } = &exec.state.objects[0] {
            assert_eq!(*energy, 100);
            assert!(spawning.is_some());
        } else {
            panic!("Expected spawn");
        }

        // Submitting another spawn request on the busy spawn must fail
        exec.resolve_actions(actions1, Vec::new());
        if let GameObject::Spawn { energy, .. } = &exec.state.objects[0] {
            assert_eq!(*energy, 100); // Energy unchanged
        }
    }
}
