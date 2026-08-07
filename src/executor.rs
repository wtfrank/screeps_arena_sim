use anyhow::{Context, Result};
use log::debug;
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
    last_action_logs: HashMap<String, serde_json::Value>,
    replay_frames: Vec<serde_json::Value>,
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
            if let GameObject::Spawn {
                next_id: nid,
                energy,
                max_energy,
                ..
            } = obj
            {
                *nid = next_id.to_string();
                next_id += 1;
                *energy = 500;
                *max_energy = 1000;
            }
            if let GameObject::Extension {
                energy, max_energy, ..
            } = obj
            {
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

        let mut executor = Self {
            state: initial_state,
            bot1_driver,
            bot2_driver,
            rules,
            bot1_crashed,
            bot2_crashed,
            debug_b1,
            debug_b2,
            next_id,
            last_action_logs: HashMap::new(),
            replay_frames: Vec::new(),
        };

        executor.state.tick = 0;
        let users_opt = Some(serde_json::json!({
            "player1": { "_id": "player1", "username": "player1", "color": "#FF3333" },
            "player2": { "_id": "player2", "username": "player2", "color": "#5555FF" }
        }));
        let tick0_frame = executor.state.to_replay_json(users_opt, &executor.last_action_logs);
        executor.replay_frames.push(tick0_frame);

        executor.state.tick = 1;

        Ok(executor)
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
                            terrain: self.state.terrain.clone(),
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
                            terrain: self.state.terrain.clone(),
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

        // 4. Resolve combat/actions and apply end-of-tick decay/fatigue recovery
        self.resolve_actions(actions1, actions2);
        self.apply_tick_decay();

        // 5. Record frame snapshot for replay emission for current tick
        let tick_frame = self.state.to_replay_json(None, &self.last_action_logs);
        self.replay_frames.push(tick_frame);

        // 6. Increment tick counter for next tick
        self.state.tick += 1;

        // 7. Check win conditions
        if let Some(res) = self.check_win_conditions_active() {
            return Ok(Some(res));
        }

        Ok(None)
    }

    pub fn current_tick(&self) -> u32 {
        self.state.tick
    }

    pub fn replay_frames(&self) -> &[serde_json::Value] {
        &self.replay_frames
    }

    pub fn save_replay<P: AsRef<std::path::Path>>(&self, path: P) -> Result<()> {
        let json_str = serde_json::to_string_pretty(&self.replay_frames)?;
        std::fs::write(path, json_str)?;
        Ok(())
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
                    if terrain == Terrain::Wall {
                        debug!(
                            "[executor] creep {} move rejected (terrain wall at {:?})",
                            id, target_pos
                        );
                    } else {
                        move_intents.insert(id.clone(), target_pos);
                    }
                } else {
                    debug!(
                        "[executor] creep {} move rejected (out of bounds {:?})",
                        id, target_pos
                    );
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
                    debug!(
                        "[executor] creep {} move rejected (contested tile {:?})",
                        creep_id, target_pos
                    );
                    move_intents.remove(&creep_id);
                    if let Some(&start) = current_positions.get(&creep_id) {
                        blocked_tiles.insert(start);
                    }
                    resolved = true;
                    continue;
                }

                // Check if target position is blocked by a static obstacle
                if blocked_tiles.contains(&target_pos) {
                    debug!(
                        "[executor] creep {} move rejected (blocked tile {:?})",
                        creep_id, target_pos
                    );
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
                        } else {
                            // Occupant creep at target_pos does not intend to move -> target is blocked!
                            debug!(
                                "[executor] creep {} move rejected (stationary creep {} at {:?})",
                                creep_id, other_id, target_pos
                            );
                            move_intents.remove(&creep_id);
                            if let Some(&start) = current_positions.get(&creep_id) {
                                blocked_tiles.insert(start);
                            }
                            resolved = true;
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
                    debug!(
                        "[executor] creep {} approved move: {:?} -> {:?}",
                        id, pos, new_pos
                    );
                    *pos = new_pos;
                    let move_parts = body
                        .iter()
                        .filter(|p| p.hits > 0 && p.part == screeps_arena::constants::Part::Move)
                        .count() as u32;
                    let carry_parts = body
                        .iter()
                        .filter(|p| p.hits > 0 && p.part == screeps_arena::constants::Part::Carry)
                        .count() as u32;
                    let store_used: u32 = store.values().sum();
                    let active_carries = (store_used.div_ceil(50)).min(carry_parts);

                    let non_carry_non_move_weight = body
                        .iter()
                        .filter(|p| {
                            p.hits > 0
                                && p.part != screeps_arena::constants::Part::Move
                                && p.part != screeps_arena::constants::Part::Carry
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

        // Filter creep actions according to action priority pipeline chains per tick per creep:
        // Chain 1: Harvest -> Attack -> Build -> RangedHeal -> Heal
        // Chain 2: RangedAttack -> RangedMassAttack -> Build -> RangedHeal
        // Chain 3: Build -> Withdraw -> Transfer -> Drop
        let chain1_order = [
            ActionId::Harvest,
            ActionId::Attack,
            ActionId::Build,
            ActionId::RangedHeal,
            ActionId::Heal,
        ];
        let chain2_order = [
            ActionId::RangedAttack,
            ActionId::RangedMassAttack,
            ActionId::Build,
            ActionId::RangedHeal,
        ];
        let chain3_order = [
            ActionId::Build,
            ActionId::Withdraw,
            ActionId::Transfer,
        ];

        // Group actions by actor_id and sort each creep's actions by chain rank index
        let mut actions_by_actor: std::collections::HashMap<String, Vec<&QueuedAction>> = std::collections::HashMap::new();
        for act in actions1.iter().chain(actions2.iter()) {
            actions_by_actor.entry(act.actor_id.clone()).or_default().push(act);
        }

        let mut valid_actions = Vec::new();

        for (_actor, mut actor_actions) in actions_by_actor {
            // Sort actions for this creep by their highest priority rank in any applicable chain
            actor_actions.sort_by_key(|act| {
                let r1 = chain1_order.iter().position(|&a| a == act.action).unwrap_or(usize::MAX);
                let r2 = chain2_order.iter().position(|&a| a == act.action).unwrap_or(usize::MAX);
                let r3 = chain3_order.iter().position(|&a| a == act.action).unwrap_or(usize::MAX);
                r1.min(r2).min(r3)
            });

            let mut chain1_used = false;
            let mut chain2_used = false;
            let mut chain3_used = false;

            for act in actor_actions {
                let in_chain1 = chain1_order.contains(&act.action);
                let in_chain2 = chain2_order.contains(&act.action);
                let in_chain3 = chain3_order.contains(&act.action);

                if in_chain1 && chain1_used {
                    continue;
                }
                if in_chain2 && chain2_used {
                    continue;
                }
                if in_chain3 && chain3_used {
                    continue;
                }

                // Range and target validation: filter out out-of-range or invalid actions upfront
                if let Some(ref tid) = act.target_id {
                    let actor_pos = self.state.objects.iter().find_map(|o| match o {
                        GameObject::Creep { id, pos, fatigue, spawning, .. } if id == &act.actor_id && *fatigue == 0 && !*spawning => Some(*pos),
                        GameObject::Tower { id, pos, .. } if id == &act.actor_id => Some(*pos),
                        _ => None,
                    });
                    let target_pos = self.state.objects.iter().find_map(|o| match o {
                        GameObject::Creep { id, pos, .. } if id == tid => Some(*pos),
                        GameObject::Spawn { id, pos, .. } if id == tid => Some(*pos),
                        GameObject::Tower { id, pos, .. } if id == tid => Some(*pos),
                        GameObject::Extension { id, pos, .. } if id == tid => Some(*pos),
                        GameObject::Container { id, pos, .. } if id == tid => Some(*pos),
                        GameObject::Rampart { id, pos, .. } if id == tid => Some(*pos),
                        GameObject::Road { id, pos, .. } if id == tid => Some(*pos),
                        GameObject::Wall { id, pos, .. } if id == tid => Some(*pos),
                        GameObject::Source { id, pos, .. } if id == tid => Some(*pos),
                        GameObject::ConstructionSite { id, pos, .. } if id == tid => Some(*pos),
                        _ => None,
                    });

                    if let (Some(apos), Some(tpos)) = (actor_pos, target_pos) {
                        let dist = (apos.x.abs_diff(tpos.x)).max(apos.y.abs_diff(tpos.y));
                        let max_range = match act.action {
                            ActionId::Attack | ActionId::Heal | ActionId::Harvest | ActionId::Transfer | ActionId::Withdraw => 1,
                            ActionId::RangedAttack | ActionId::RangedHeal | ActionId::Build => 3,
                            _ => u8::MAX,
                        };
                        if dist > max_range {
                            continue; // Reject out-of-range action upfront!
                        }
                    } else if act.action != ActionId::RangedMassAttack && act.action != ActionId::SpawnCreep {
                        continue; // Target missing
                    }
                }

                if in_chain1 {
                    chain1_used = true;
                }
                if in_chain2 {
                    chain2_used = true;
                }
                if in_chain3 {
                    chain3_used = true;
                }

                valid_actions.push(act);
            }
        }

        // Resolve Harvest, Transfer, and Withdraw actions
        for act in &valid_actions {
            match act.action {
                ActionId::Harvest => {
                    let creep_id = &act.actor_id;
                    let target_id = act.target_id.as_deref();

                    let mut harvest_pos = None;
                    let mut harvest_power = 0;
                    let mut creep_carry_avail = 0;

                    for obj in &self.state.objects {
                        if let GameObject::Creep {
                            id, pos, body, store, fatigue, spawning, ..
                        } = obj
                        {
                            if id == creep_id && *fatigue == 0 && !*spawning {
                                harvest_pos = Some(*pos);
                                let work_parts = body
                                    .iter()
                                    .filter(|p| p.hits > 0 && p.part == screeps_arena::constants::Part::Work)
                                    .count() as u32;
                                harvest_power = work_parts * 2;

                                let carry_parts = body
                                    .iter()
                                    .filter(|p| p.hits > 0 && p.part == screeps_arena::constants::Part::Carry)
                                    .count() as u32;
                                let max_capacity = carry_parts * 50;
                                let current_used: u32 = store.values().sum();
                                creep_carry_avail = max_capacity.saturating_sub(current_used);
                                break;
                            }
                        }
                    }

                    if let (Some(cpos), Some(tid)) = (harvest_pos, target_id) {
                        let mut harvested_amount = 0;
                        for obj in &mut self.state.objects {
                            if let GameObject::Source { id, pos, energy, .. } = obj {
                                if id == tid && pos.x.abs_diff(cpos.x) <= 1 && pos.y.abs_diff(cpos.y) <= 1 {
                                    harvested_amount = (*energy).min(harvest_power).min(creep_carry_avail);
                                    *energy -= harvested_amount;
                                    break;
                                }
                            }
                        }

                        if harvested_amount > 0 {
                            for obj in &mut self.state.objects {
                                if let GameObject::Creep { id, store, .. } = obj {
                                    if id == creep_id {
                                        *store.entry(screeps_arena::constants::ResourceType::Energy).or_insert(0) += harvested_amount;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                ActionId::Transfer => {
                    let creep_id = &act.actor_id;
                    let target_id = act.target_id.as_deref();
                    let req_amount = if act.arg2 > 0 { act.arg2 as u32 } else { u32::MAX };

                    let mut creep_pos = None;
                    let mut creep_energy = 0;

                    for obj in &self.state.objects {
                        if let GameObject::Creep {
                            id, pos, store, fatigue, spawning, ..
                        } = obj
                        {
                            if id == creep_id && *fatigue == 0 && !*spawning {
                                creep_pos = Some(*pos);
                                creep_energy = *store.get(&screeps_arena::constants::ResourceType::Energy).unwrap_or(&0);
                                break;
                            }
                        }
                    }

                    if let (Some(cpos), Some(tid)) = (creep_pos, target_id) {
                        let mut transferred = 0;
                        for obj in &mut self.state.objects {
                            match obj {
                                GameObject::Spawn { id, pos, energy, max_energy, .. } if id == tid => {
                                    if pos.x.abs_diff(cpos.x) <= 1 && pos.y.abs_diff(cpos.y) <= 1 {
                                        let space = max_energy.saturating_sub(*energy);
                                        transferred = creep_energy.min(req_amount).min(space);
                                        *energy += transferred;
                                    }
                                }
                                GameObject::Extension { id, pos, energy, max_energy, .. } if id == tid => {
                                    if pos.x.abs_diff(cpos.x) <= 1 && pos.y.abs_diff(cpos.y) <= 1 {
                                        let space = max_energy.saturating_sub(*energy);
                                        transferred = creep_energy.min(req_amount).min(space);
                                        *energy += transferred;
                                    }
                                }
                                GameObject::Tower { id, pos, energy, max_energy, .. } if id == tid => {
                                    if pos.x.abs_diff(cpos.x) <= 1 && pos.y.abs_diff(cpos.y) <= 1 {
                                        let space = max_energy.saturating_sub(*energy);
                                        transferred = creep_energy.min(req_amount).min(space);
                                        *energy += transferred;
                                    }
                                }
                                GameObject::Container { id, pos, energy, max_energy, .. } if id == tid => {
                                    if pos.x.abs_diff(cpos.x) <= 1 && pos.y.abs_diff(cpos.y) <= 1 {
                                        let space = max_energy.saturating_sub(*energy);
                                        transferred = creep_energy.min(req_amount).min(space);
                                        *energy += transferred;
                                    }
                                }
                                _ => {}
                            }
                        }

                        if transferred > 0 {
                            for obj in &mut self.state.objects {
                                if let GameObject::Creep { id, store, .. } = obj {
                                    if id == creep_id {
                                        if let Some(e) = store.get_mut(&screeps_arena::constants::ResourceType::Energy) {
                                            *e -= transferred;
                                        }
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                ActionId::Withdraw => {
                    let creep_id = &act.actor_id;
                    let target_id = act.target_id.as_deref();
                    let req_amount = if act.arg2 > 0 { act.arg2 as u32 } else { u32::MAX };

                    let mut creep_pos = None;
                    let mut creep_carry_avail = 0;

                    for obj in &self.state.objects {
                        if let GameObject::Creep {
                            id, pos, body, store, fatigue, spawning, ..
                        } = obj
                        {
                            if id == creep_id && *fatigue == 0 && !*spawning {
                                creep_pos = Some(*pos);
                                let carry_parts = body
                                    .iter()
                                    .filter(|p| p.hits > 0 && p.part == screeps_arena::constants::Part::Carry)
                                    .count() as u32;
                                let max_capacity = carry_parts * 50;
                                let current_used: u32 = store.values().sum();
                                creep_carry_avail = max_capacity.saturating_sub(current_used);
                                break;
                            }
                        }
                    }

                    if let (Some(cpos), Some(tid)) = (creep_pos, target_id) {
                        let mut withdrawn = 0;
                        for obj in &mut self.state.objects {
                            match obj {
                                GameObject::Spawn { id, pos, energy, .. } if id == tid => {
                                    if pos.x.abs_diff(cpos.x) <= 1 && pos.y.abs_diff(cpos.y) <= 1 {
                                        withdrawn = (*energy).min(req_amount).min(creep_carry_avail);
                                        *energy -= withdrawn;
                                    }
                                }
                                GameObject::Extension { id, pos, energy, .. } if id == tid => {
                                    if pos.x.abs_diff(cpos.x) <= 1 && pos.y.abs_diff(cpos.y) <= 1 {
                                        withdrawn = (*energy).min(req_amount).min(creep_carry_avail);
                                        *energy -= withdrawn;
                                    }
                                }
                                GameObject::Tower { id, pos, energy, .. } if id == tid => {
                                    if pos.x.abs_diff(cpos.x) <= 1 && pos.y.abs_diff(cpos.y) <= 1 {
                                        withdrawn = (*energy).min(req_amount).min(creep_carry_avail);
                                        *energy -= withdrawn;
                                    }
                                }
                                GameObject::Container { id, pos, energy, .. } if id == tid => {
                                    if pos.x.abs_diff(cpos.x) <= 1 && pos.y.abs_diff(cpos.y) <= 1 {
                                        withdrawn = (*energy).min(req_amount).min(creep_carry_avail);
                                        *energy -= withdrawn;
                                    }
                                }
                                _ => {}
                            }
                        }

                        if withdrawn > 0 {
                            for obj in &mut self.state.objects {
                                if let GameObject::Creep { id, store, .. } = obj {
                                    if id == creep_id {
                                        *store.entry(screeps_arena::constants::ResourceType::Energy).or_insert(0) += withdrawn;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                ActionId::Build => {
                    let creep_id = &act.actor_id;
                    let target_id = act.target_id.as_deref();

                    let mut build_pos = None;
                    let mut build_power = 0;
                    let mut creep_energy = 0;

                    for obj in &self.state.objects {
                        if let GameObject::Creep {
                            id, pos, body, store, fatigue, spawning, ..
                        } = obj
                        {
                            if id == creep_id && *fatigue == 0 && !*spawning {
                                build_pos = Some(*pos);
                                let work_parts = body
                                    .iter()
                                    .filter(|p| p.hits > 0 && p.part == screeps_arena::constants::Part::Work)
                                    .count() as u32;
                                build_power = work_parts * 5;
                                creep_energy = *store.get(&screeps_arena::constants::ResourceType::Energy).unwrap_or(&0);
                                break;
                            }
                        }
                    }

                    if let (Some(cpos), Some(tid)) = (build_pos, target_id) {
                        let mut applied_build = 0;
                        for obj in &mut self.state.objects {
                            if let GameObject::ConstructionSite { id, pos, progress, progress_total, .. } = obj {
                                if id == tid && pos.x.abs_diff(cpos.x) <= 3 && pos.y.abs_diff(cpos.y) <= 3 {
                                    let remaining_needed = progress_total.saturating_sub(*progress);
                                    applied_build = build_power.min(creep_energy).min(remaining_needed);
                                    *progress += applied_build;
                                    break;
                                }
                            }
                        }

                        if applied_build > 0 {
                            for obj in &mut self.state.objects {
                                if let GameObject::Creep { id, store, .. } = obj {
                                    if id == creep_id {
                                        if let Some(e) = store.get_mut(&screeps_arena::constants::ResourceType::Energy) {
                                            *e -= applied_build;
                                        }
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Resolve Attack, RangedAttack, RangedMassAttack, Heal, and RangedHeal actions
        let mut damage_map = HashMap::new();
        let mut heal_map = HashMap::new();

        for act in &valid_actions {
            let actor_id = &act.actor_id;

            // Retrieve actor position, owner, and active body parts
            let mut actor_info = None;
            for obj in &self.state.objects {
                if let GameObject::Creep {
                    id, pos, owner, body, fatigue, spawning, ..
                } = obj
                {
                    if id == actor_id && *fatigue == 0 && !*spawning {
                        actor_info = Some((*pos, *owner, body.clone()));
                        break;
                    }
                }
            }

            let (apos, aowner, abody) = match actor_info {
                Some(info) => info,
                None => continue,
            };

            match act.action {
                ActionId::Attack => {
                    if let Some(ref target_id) = act.target_id {
                        let active_parts = abody
                            .iter()
                            .filter(|p| p.hits > 0 && p.part == screeps_arena::constants::Part::Attack)
                            .count() as u32;
                        let power = if active_parts > 0 { active_parts * 30 } else { 30 };

                        for obj in &self.state.objects {
                            let tpos = match obj {
                                GameObject::Creep { id, pos, .. } if id == target_id => Some(*pos),
                                GameObject::Spawn { id, pos, .. } if id == target_id => Some(*pos),
                                GameObject::Tower { id, pos, .. } if id == target_id => Some(*pos),
                                GameObject::Extension { id, pos, .. } if id == target_id => Some(*pos),
                                GameObject::Container { id, pos, .. } if id == target_id => Some(*pos),
                                GameObject::Rampart { id, pos, .. } if id == target_id => Some(*pos),
                                GameObject::Road { id, pos, .. } if id == target_id => Some(*pos),
                                GameObject::Wall { id, pos, .. } if id == target_id => Some(*pos),
                                _ => None,
                            };
                            if let Some(tp) = tpos {
                                if apos.x.abs_diff(tp.x) <= 1 && apos.y.abs_diff(tp.y) <= 1 {
                                    *damage_map.entry(target_id.clone()).or_insert(0) += power;
                                }
                                break;
                            }
                        }
                    }
                }
                ActionId::RangedAttack => {
                    if let Some(ref target_id) = act.target_id {
                        let active_parts = abody
                            .iter()
                            .filter(|p| p.hits > 0 && p.part == screeps_arena::constants::Part::RangedAttack)
                            .count() as u32;
                        let power = if active_parts > 0 { active_parts * 10 } else { 10 };

                        for obj in &self.state.objects {
                            let tpos = match obj {
                                GameObject::Creep { id, pos, .. } if id == target_id => Some(*pos),
                                GameObject::Spawn { id, pos, .. } if id == target_id => Some(*pos),
                                GameObject::Tower { id, pos, .. } if id == target_id => Some(*pos),
                                GameObject::Extension { id, pos, .. } if id == target_id => Some(*pos),
                                GameObject::Container { id, pos, .. } if id == target_id => Some(*pos),
                                GameObject::Rampart { id, pos, .. } if id == target_id => Some(*pos),
                                GameObject::Road { id, pos, .. } if id == target_id => Some(*pos),
                                GameObject::Wall { id, pos, .. } if id == target_id => Some(*pos),
                                _ => None,
                            };
                            if let Some(tp) = tpos {
                                let dist = (apos.x.abs_diff(tp.x)).max(apos.y.abs_diff(tp.y));
                                if dist <= 3 {
                                    *damage_map.entry(target_id.clone()).or_insert(0) += power;
                                }
                                break;
                            }
                        }
                    }
                }
                ActionId::RangedMassAttack => {
                    let active_parts = abody
                        .iter()
                        .filter(|p| p.hits > 0 && p.part == screeps_arena::constants::Part::RangedAttack)
                        .count() as u32;
                    let num_parts = if active_parts > 0 { active_parts } else { 1 };

                    // Deal area damage to hostiles within Chebyshev distance 3 (range 1: 10/part, range 2: 4/part, range 3: 1/part)
                    for obj in &self.state.objects {
                        let (tid, tpos, is_hostile) = match obj {
                            GameObject::Creep { id, pos, owner, .. } => (id.clone(), *pos, *owner != aowner),
                            GameObject::Spawn { id, pos, owner, .. } => (id.clone(), *pos, *owner != aowner),
                            GameObject::Tower { id, pos, owner, .. } => (id.clone(), *pos, *owner != aowner),
                            GameObject::Extension { id, pos, owner, .. } => (id.clone(), *pos, *owner != aowner),
                            GameObject::Rampart { id, pos, owner, .. } => (id.clone(), *pos, *owner != aowner),
                            _ => continue,
                        };

                        if is_hostile {
                            let dist = (apos.x.abs_diff(tpos.x)).max(apos.y.abs_diff(tpos.y));
                            let dmg_per_part = match dist {
                                1 => 10,
                                2 => 4,
                                3 => 1,
                                _ => 0,
                            };
                            if dmg_per_part > 0 {
                                *damage_map.entry(tid).or_insert(0) += num_parts * dmg_per_part;
                            }
                        }
                    }
                }
                ActionId::Heal => {
                    if let Some(ref target_id) = act.target_id {
                        let active_parts = abody
                            .iter()
                            .filter(|p| p.hits > 0 && p.part == screeps_arena::constants::Part::Heal)
                            .count() as u32;
                        let power = if active_parts > 0 { active_parts * 12 } else { 12 };

                        for obj in &self.state.objects {
                            if let GameObject::Creep { id, pos, .. } = obj {
                                if id == target_id && apos.x.abs_diff(pos.x) <= 1 && apos.y.abs_diff(pos.y) <= 1 {
                                    *heal_map.entry(target_id.clone()).or_insert(0) += power;
                                    break;
                                }
                            }
                        }
                    }
                }
                ActionId::RangedHeal => {
                    if let Some(ref target_id) = act.target_id {
                        let active_parts = abody
                            .iter()
                            .filter(|p| p.hits > 0 && p.part == screeps_arena::constants::Part::Heal)
                            .count() as u32;
                        let power = if active_parts > 0 { active_parts * 4 } else { 4 };

                        for obj in &self.state.objects {
                            if let GameObject::Creep { id, pos, .. } = obj {
                                if id == target_id {
                                    let dist = (apos.x.abs_diff(pos.x)).max(apos.y.abs_diff(pos.y));
                                    if dist <= 3 {
                                        *heal_map.entry(target_id.clone()).or_insert(0) += power;
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Build actionLog objects for visual actions recorded this tick
        let mut tick_action_logs: HashMap<String, serde_json::Value> = HashMap::new();

        let get_obj_pos = |target_id: &str, objects: &[GameObject]| -> Option<Position> {
            objects.iter().find_map(|o| match o {
                GameObject::Creep { id, pos, .. } if id == target_id => Some(*pos),
                GameObject::Spawn { id, pos, .. } if id == target_id => Some(*pos),
                GameObject::Tower { id, pos, .. } if id == target_id => Some(*pos),
                GameObject::Extension { id, pos, .. } if id == target_id => Some(*pos),
                GameObject::Container { id, pos, .. } if id == target_id => Some(*pos),
                GameObject::Rampart { id, pos, .. } if id == target_id => Some(*pos),
                GameObject::Road { id, pos, .. } if id == target_id => Some(*pos),
                GameObject::Wall { id, pos, .. } if id == target_id => Some(*pos),
                GameObject::Source { id, pos, .. } if id == target_id => Some(*pos),
                GameObject::ConstructionSite { id, pos, .. } if id == target_id => Some(*pos),
                _ => None,
            })
        };

        for act in &valid_actions {
            let actor_id = &act.actor_id;
            let target_id_opt = act.target_id.as_deref();

            let actor_pos = match get_obj_pos(actor_id, &self.state.objects) {
                Some(p) => p,
                None => continue,
            };

            match act.action {
                ActionId::Heal => {
                    if let Some(target_id) = target_id_opt {
                        if let Some(tp) = get_obj_pos(target_id, &self.state.objects) {
                            let entry = tick_action_logs.entry(actor_id.clone()).or_insert_with(|| serde_json::json!({}));
                            entry["heal"] = serde_json::json!({ "x": tp.x, "y": tp.y });

                            let target_entry = tick_action_logs.entry(target_id.to_string()).or_insert_with(|| serde_json::json!({}));
                            target_entry["healed"] = serde_json::json!({ "x": actor_pos.x, "y": actor_pos.y });
                        }
                    }
                }
                ActionId::RangedHeal => {
                    if let Some(target_id) = target_id_opt {
                        if let Some(tp) = get_obj_pos(target_id, &self.state.objects) {
                            let entry = tick_action_logs.entry(actor_id.clone()).or_insert_with(|| serde_json::json!({}));
                            entry["rangedHeal"] = serde_json::json!({ "x": tp.x, "y": tp.y });
                        }
                    }
                }
                ActionId::Attack => {
                    if let Some(target_id) = target_id_opt {
                        if let Some(tp) = get_obj_pos(target_id, &self.state.objects) {
                            let entry = tick_action_logs.entry(actor_id.clone()).or_insert_with(|| serde_json::json!({}));
                            entry["attack"] = serde_json::json!({ "x": tp.x, "y": tp.y });
                        }
                    }
                }
                ActionId::RangedAttack => {
                    if let Some(target_id) = target_id_opt {
                        if let Some(tp) = get_obj_pos(target_id, &self.state.objects) {
                            let entry = tick_action_logs.entry(actor_id.clone()).or_insert_with(|| serde_json::json!({}));
                            entry["rangedAttack"] = serde_json::json!({ "x": tp.x, "y": tp.y });
                        }
                    }
                }
                ActionId::RangedMassAttack => {
                    let entry = tick_action_logs.entry(actor_id.clone()).or_insert_with(|| serde_json::json!({}));
                    entry["rangedMassAttack"] = serde_json::json!({});
                }
                ActionId::Harvest => {
                    if let Some(target_id) = target_id_opt {
                        if let Some(tp) = get_obj_pos(target_id, &self.state.objects) {
                            let entry = tick_action_logs.entry(actor_id.clone()).or_insert_with(|| serde_json::json!({}));
                            entry["harvest"] = serde_json::json!({ "x": tp.x, "y": tp.y });
                        }
                    }
                }
                ActionId::Transfer => {
                    if let Some(target_id) = target_id_opt {
                        if let Some(tp) = get_obj_pos(target_id, &self.state.objects) {
                            let entry = tick_action_logs.entry(actor_id.clone()).or_insert_with(|| serde_json::json!({}));
                            entry["transfer"] = serde_json::json!({ "x": tp.x, "y": tp.y });
                        }
                    }
                }
                ActionId::Withdraw => {
                    if let Some(target_id) = target_id_opt {
                        if let Some(tp) = get_obj_pos(target_id, &self.state.objects) {
                            let entry = tick_action_logs.entry(actor_id.clone()).or_insert_with(|| serde_json::json!({}));
                            entry["withdraw"] = serde_json::json!({ "x": tp.x, "y": tp.y });
                        }
                    }
                }
                ActionId::Build => {
                    if let Some(target_id) = target_id_opt {
                        if let Some(tp) = get_obj_pos(target_id, &self.state.objects) {
                            let entry = tick_action_logs.entry(actor_id.clone()).or_insert_with(|| serde_json::json!({}));
                            entry["build"] = serde_json::json!({ "x": tp.x, "y": tp.y });
                        }
                    }
                }
                _ => {}
            }
        }

        self.last_action_logs = tick_action_logs;

        // Sum all healing and damage intents per creep/structure during the tick
        for obj in &mut self.state.objects {
            match obj {
                GameObject::Creep { id, hits, max_hits, body, .. } => {
                    let total_damage = damage_map.get(id).copied().unwrap_or(0);
                    let total_heal = heal_map.get(id).copied().unwrap_or(0);

                    if total_heal >= total_damage {
                        let net_heal = total_heal - total_damage;
                        if net_heal > 0 {
                            let mut remaining_heal = net_heal;
                            for part in body.iter_mut().rev() {
                                if remaining_heal == 0 {
                                    break;
                                }
                                let missing = 100u32.saturating_sub(part.hits);
                                let h = missing.min(remaining_heal);
                                part.hits += h;
                                remaining_heal -= h;
                            }
                            *hits = body.iter().map(|p| p.hits).sum::<u32>().min(*max_hits);
                        }
                    } else {
                        let net_damage = total_damage - total_heal;
                        let mut remaining_dmg = net_damage;
                        for part in body.iter_mut() {
                            if remaining_dmg == 0 {
                                break;
                            }
                            let d = part.hits.min(remaining_dmg);
                            part.hits -= d;
                            remaining_dmg -= d;
                        }
                        *hits = body.iter().map(|p| p.hits).sum();
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
                GameObject::Extension { id, hits, .. } => {
                    if let Some(&dmg) = damage_map.get(id) {
                        *hits = hits.saturating_sub(dmg);
                    }
                }
                GameObject::Container { id, hits, .. } => {
                    if let Some(&dmg) = damage_map.get(id) {
                        *hits = hits.saturating_sub(dmg);
                    }
                }
                GameObject::Rampart { id, hits, .. } => {
                    if let Some(&dmg) = damage_map.get(id) {
                        *hits = hits.saturating_sub(dmg);
                    }
                }
                GameObject::Road { id, hits, .. } => {
                    if let Some(&dmg) = damage_map.get(id) {
                        *hits = hits.saturating_sub(dmg);
                    }
                }
                GameObject::Wall { id, hits, .. } => {
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

                // Decode body parts from act.arg2 (nibble packed 4-bit enum values)
                let mut decoded_body = Vec::new();
                for i in 0..body_len.min(16) {
                    let val = ((act.arg2 >> (i * 4)) & 0xF) as u8;
                    let part = match val {
                        1 => screeps_arena::constants::Part::Move,
                        2 => screeps_arena::constants::Part::Work,
                        3 => screeps_arena::constants::Part::Carry,
                        4 => screeps_arena::constants::Part::Attack,
                        5 => screeps_arena::constants::Part::RangedAttack,
                        6 => screeps_arena::constants::Part::Tough,
                        7 => screeps_arena::constants::Part::Heal,
                        _ => screeps_arena::constants::Part::Move,
                    };
                    decoded_body.push(part);
                }

                // Calculate energy cost strictly from decoded body parts
                let energy_cost: u32 = decoded_body.iter().map(|p| p.cost()).sum();

                // Verify spawn ownership and presence, checking energy within SPAWN_RANGE
                let mut spawn_pos = None;
                let mut available_energy = 0;
                let range = screeps_arena::constants::SPAWN_RANGE as u8;

                for obj in &self.state.objects {
                    if let GameObject::Spawn {
                        id,
                        owner: o,
                        pos,
                        spawning,
                        ..
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
                                available_energy += energy;
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
                                available_energy += energy;
                            }
                            _ => {}
                        }
                    }

                    if available_energy >= energy_cost {
                        let mut remaining_needed = energy_cost;

                        // Collect all energy structures (spawns & extensions within SPAWN_RANGE) sorted by Manhattan distance
                        let mut energy_candidates: Vec<(usize, u32)> = self
                            .state
                            .objects
                            .iter()
                            .enumerate()
                            .filter_map(|(idx, obj)| match obj {
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
                                    let dist = spos.x.abs_diff(pos.x) as u32 + spos.y.abs_diff(pos.y) as u32;
                                    Some((idx, dist))
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
                                    let dist = spos.x.abs_diff(pos.x) as u32 + spos.y.abs_diff(pos.y) as u32;
                                    Some((idx, dist))
                                }
                                _ => None,
                            })
                            .collect();

                        // Sort structures by Manhattan distance to the spawning position
                        energy_candidates.sort_by_key(|&(_, dist)| dist);

                        for (idx, _) in energy_candidates {
                            if remaining_needed == 0 {
                                break;
                            }
                            match &mut self.state.objects[idx] {
                                GameObject::Extension { energy, .. } | GameObject::Spawn { energy, .. } => {
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
                        let need_time = body_len.max(1) * 3;
                        let spawn_time = self.state.tick + need_time;

                        let spawn_progress = crate::models::SpawningProgress {
                            creep_id: new_creep_id.clone(),
                            need_time,
                            remaining_time: spawn_time,
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
                        let body_part_states: Vec<crate::models::BodyPartState> = decoded_body
                            .into_iter()
                            .map(|part| crate::models::BodyPartState { part, hits: 100 })
                            .collect();
                        let max_hits = (body_part_states.len() as u32) * 100;

                        self.state.objects.push(GameObject::Creep {
                            id: new_creep_id,
                            pos: spos,
                            hits: max_hits,
                            max_hits,
                            owner,
                            fatigue: 0,
                            spawning: true,
                            body: body_part_states,
                            store: HashMap::new(),
                        });
                    }
                }
            }
        }

        // Remove destroyed units and structures
        self.state.objects.retain(|o| match o {
            GameObject::Creep { hits, .. } => *hits > 0,
            GameObject::Spawn { hits, .. } => *hits > 0,
            GameObject::Tower { hits, .. } => *hits > 0,
            GameObject::Extension { hits, .. } => *hits > 0,
            GameObject::Rampart { hits, .. } => *hits > 0,
            GameObject::Container { hits, .. } => *hits > 0,
            GameObject::Road { hits, .. } => *hits > 0,
            GameObject::Wall { hits, .. } => *hits > 0,
            _ => true,
        });
    }

    fn apply_tick_decay(&mut self) {
        // 1. Check creeps over capacity (due to disabled CARRY parts) and drop excess resources
        let mut new_dropped_resources = Vec::new();

        for obj in &mut self.state.objects {
            if let GameObject::Creep {
                id: _id, pos, hits, body, store, ..
            } = obj
            {
                if *hits > 0 {
                    let active_carry_parts = body
                        .iter()
                        .filter(|p| p.hits > 0 && p.part == screeps_arena::constants::Part::Carry)
                        .count() as u32;
                    let max_capacity = active_carry_parts * 50;
                    let current_used: u32 = store.values().sum();

                    if current_used > max_capacity {
                        let mut excess = current_used - max_capacity;
                        for (res_type, amount) in store.iter_mut() {
                            if excess == 0 {
                                break;
                            }
                            let drop_amt = (*amount).min(excess);
                            *amount -= drop_amt;
                            excess -= drop_amt;

                            let res_type_str = format!("{:?}", res_type);
                            new_dropped_resources.push(GameObject::Resource {
                                id: format!("res_{}", self.next_id),
                                pos: *pos,
                                amount: drop_amt,
                                resource_type: res_type_str,
                            });
                            self.next_id += 1;
                        }
                    }
                }
            }
        }

        // Push dropped excess resources to game objects
        self.state.objects.extend(new_dropped_resources);

        // Process decay on existing ground Resource objects at ceil(amount / 1000) per tick
        for obj in &mut self.state.objects {
            if let GameObject::Resource { amount, .. } = obj {
                let decay = ((*amount as f64) / 1000.0).ceil() as u32;
                *amount = amount.saturating_sub(decay);
            }
        }

        // Process completed ConstructionSite objects (progress >= progress_total)
        let mut completed_structures = Vec::new();
        self.state.objects.retain_mut(|o| {
            if let GameObject::ConstructionSite { id, pos, owner, progress, progress_total, structure_type } = o {
                if *progress >= *progress_total {
                    let new_obj = match structure_type {
                        crate::models::StructureType::Spawn => GameObject::Spawn {
                            id: id.clone(),
                            pos: *pos,
                            hits: 5000,
                            max_hits: 5000,
                            owner: *owner,
                            energy: 0,
                            max_energy: 1000,
                            spawning: None,
                            next_id: format!("{}_c1", id),
                            directions: screeps_arena::constants::DEFAULT_SPAWN_DIRECTIONS.to_vec(),
                        },
                        crate::models::StructureType::Extension => GameObject::Extension {
                            id: id.clone(),
                            pos: *pos,
                            hits: 500,
                            max_hits: 500,
                            owner: *owner,
                            energy: 0,
                            max_energy: 100,
                        },
                        crate::models::StructureType::Tower => GameObject::Tower {
                            id: id.clone(),
                            pos: *pos,
                            hits: 3000,
                            max_hits: 3000,
                            owner: *owner,
                            energy: 0,
                            max_energy: 1000,
                        },
                        crate::models::StructureType::Container => GameObject::Container {
                            id: id.clone(),
                            pos: *pos,
                            hits: 2500,
                            max_hits: 2500,
                            energy: 0,
                            max_energy: 2000,
                        },
                        crate::models::StructureType::Rampart => GameObject::Rampart {
                            id: id.clone(),
                            pos: *pos,
                            hits: 10000,
                            max_hits: 10000,
                            owner: *owner,
                            controlled_by: None,
                        },
                        crate::models::StructureType::Road => GameObject::Road {
                            id: id.clone(),
                            pos: *pos,
                            hits: 5000,
                            max_hits: 5000,
                        },
                        crate::models::StructureType::Wall => GameObject::Wall {
                            id: id.clone(),
                            pos: *pos,
                            hits: 10000,
                            max_hits: 10000,
                        },
                    };
                    completed_structures.push(new_obj);
                    return false; // Remove completed ConstructionSite
                }
            }
            true
        });
        self.state.objects.extend(completed_structures);

        // Remove destroyed units, structures, and depleted ground resources
        self.state.objects.retain(|o| match o {
            GameObject::Creep { hits, .. } => *hits > 0,
            GameObject::Spawn { hits, .. } => *hits > 0,
            GameObject::Tower { hits, .. } => *hits > 0,
            GameObject::Extension { hits, .. } => *hits > 0,
            GameObject::Rampart { hits, .. } => *hits > 0,
            GameObject::Container { hits, .. } => *hits > 0,
            GameObject::Road { hits, .. } => *hits > 0,
            GameObject::Wall { hits, .. } => *hits > 0,
            GameObject::Resource { amount, .. } => *amount > 0,
            _ => true,
        });

        // 2. Process fatigue decay and spawn energy regeneration
        for obj in &mut self.state.objects {
            if let GameObject::Creep { fatigue, body, spawning, .. } = obj {
                if !*spawning {
                    let move_parts = body
                        .iter()
                        .filter(|p| p.hits > 0 && p.part == screeps_arena::constants::Part::Move)
                        .count() as u32;
                    let fatigue_reduction = move_parts * 2;
                    *fatigue = fatigue.saturating_sub(fatigue_reduction as u8);
                }
            } else if let GameObject::Spawn { energy, max_energy, .. } = obj {
                if *energy < *max_energy {
                    *energy += 1;
                }
            }
        }

        self.process_spawn_completions();
    }

    fn process_spawn_completions(&mut self) {
        let mut ready_spawns = Vec::new();

        for obj in &mut self.state.objects {
            if let GameObject::Spawn {
                id,
                pos,
                spawning,
                directions,
                ..
            } = obj
            {
                if let Some(progress) = spawning {
                    if self.state.tick + 1 >= progress.remaining_time {
                        ready_spawns.push((id.clone(), *pos, progress.creep_id.clone(), directions.clone()));
                    }
                }
            }
        }

        // For creeps finished spawning, attempt placement in an adjacent non-blocked spot before setting spawning = false
        for (spawn_id, spawn_pos, creep_id, spawn_directions) in ready_spawns {
            let mut free_spot = None;

            for dir in spawn_directions {
                let (dx, dy) = match dir {
                    screeps_arena::constants::Direction::Top => (0i32, -1i32),
                    screeps_arena::constants::Direction::TopRight => (1i32, -1i32),
                    screeps_arena::constants::Direction::Right => (1i32, 0i32),
                    screeps_arena::constants::Direction::BottomRight => (1i32, 1i32),
                    screeps_arena::constants::Direction::Bottom => (0i32, 1i32),
                    screeps_arena::constants::Direction::BottomLeft => (-1i32, 1i32),
                    screeps_arena::constants::Direction::Left => (-1i32, 0i32),
                    screeps_arena::constants::Direction::TopLeft => (-1i32, -1i32),
                };

                let nx = spawn_pos.x as i32 + dx;
                let ny = spawn_pos.y as i32 + dy;

                if nx >= 0
                    && nx < self.state.width as i32
                    && ny >= 0
                    && ny < self.state.height as i32
                {
                    let candidate = Position {
                        x: nx as u8,
                        y: ny as u8,
                    };
                    if self.state.terrain[candidate.x as usize][candidate.y as usize] == Terrain::Wall {
                        continue;
                    }

                    // Check if tile is occupied by static structure or another non-spawning creep
                    let occupied = self.state.objects.iter().any(|o| match o {
                        GameObject::Creep { pos, spawning, .. } => *pos == candidate && !*spawning,
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

            if let Some(new_pos) = free_spot {
                // Move creep to free adjacent spot and set spawning = false
                for obj in &mut self.state.objects {
                    if let GameObject::Creep {
                        id, pos, spawning, ..
                    } = obj
                    {
                        if id == &creep_id {
                            *pos = new_pos;
                            *spawning = false;
                        }
                    }
                }

                // Clear spawning state from spawn
                for obj in &mut self.state.objects {
                    if let GameObject::Spawn { id, spawning, .. } = obj {
                        if id == &spawn_id {
                            *spawning = None;
                        }
                    }
                }
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
                    body,
                    ..
                } => {
                    let my = if is_bot1 {
                        *owner == Owner::Bot1
                    } else {
                        *owner == Owner::Bot2
                    };
                    let mock_body: Vec<screeps_arena::objects::BodyPart> = body
                        .iter()
                        .map(|p| screeps_arena::objects::BodyPart {
                            part: p.part,
                            hits: p.hits,
                        })
                        .collect();
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
                        body: mock_body,
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
                    directions,
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
                        directions: directions.clone(),
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
                    ..
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
                    ..
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

    fn get_mock_owned_structures(
        &self,
        is_bot1: bool,
    ) -> Vec<screeps_arena::objects::OwnedStructure> {
        let mut result = Vec::new();

        for obj in &self.state.objects {
            match obj {
                GameObject::Spawn { id, pos, owner, .. }
                | GameObject::Tower { id, pos, owner, .. }
                | GameObject::Extension { id, pos, owner, .. } => {
                    let my = if is_bot1 {
                        *owner == Owner::Bot1
                    } else {
                        *owner == Owner::Bot2
                    };
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
            next_id: 1,
            last_action_logs: HashMap::new(),
            replay_frames: Vec::new(),
        }
    }

    #[test]
    fn test_fatigue_decay() {
        let mut exec = create_test_executor();
        exec.state.objects.push(GameObject::Creep {
            id: "creep1".to_string(),
            pos: Position { x: 10, y: 10 },
            hits: 200,
            max_hits: 200,
            owner: Owner::Bot1,
            fatigue: 5,
            spawning: false,
            body: vec![
                crate::models::BodyPartState { part: screeps_arena::constants::Part::Move, hits: 100 },
                crate::models::BodyPartState { part: screeps_arena::constants::Part::Move, hits: 100 },
            ],
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
            body: vec![crate::models::BodyPartState { part: screeps_arena::constants::Part::Move, hits: 100 }],
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
            body: vec![crate::models::BodyPartState { part: screeps_arena::constants::Part::Move, hits: 100 }],
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
            body: vec![crate::models::BodyPartState { part: screeps_arena::constants::Part::Move, hits: 100 }],
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
            body: vec![crate::models::BodyPartState { part: screeps_arena::constants::Part::Move, hits: 100 }],
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
            body: vec![crate::models::BodyPartState { part: screeps_arena::constants::Part::Move, hits: 100 }],
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
            directions: screeps_arena::constants::DEFAULT_SPAWN_DIRECTIONS.to_vec(),
        });

        // Spawn 1 creep with Move + RangedAttack (encoded 0x51, cost 200)
        let actions1 = vec![QueuedAction {
            actor_id: "spawn1".to_string(),
            action: ActionId::SpawnCreep,
            target_id: None,
            arg1: 2,
            arg2: 0x51,
        }];

        exec.resolve_actions(actions1.clone(), Vec::new());

        // Spawn energy should be deducted (300 -> 100) and spawning state set
        if let GameObject::Spawn {
            energy, spawning, ..
        } = &exec.state.objects[0]
        {
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

    #[test]
    fn test_spawn_energy_tick_regeneration() {
        let mut exec = create_test_executor();
        exec.state.objects.push(GameObject::Spawn {
            id: "spawn1".to_string(),
            pos: Position { x: 10, y: 10 },
            hits: 5000,
            max_hits: 5000,
            owner: Owner::Bot1,
            energy: 400,
            max_energy: 1000,
            spawning: None,
            next_id: "creep1".to_string(),
            directions: screeps_arena::constants::DEFAULT_SPAWN_DIRECTIONS.to_vec(),
        });

        exec.apply_tick_decay();
        if let GameObject::Spawn { energy, .. } = &exec.state.objects[0] {
            assert_eq!(*energy, 401);
        } else {
            panic!("Expected spawn");
        }
    }

    #[test]
    fn test_destroyed_rampart_removal() {
        let mut exec = create_test_executor();
        exec.state.objects.push(GameObject::Rampart {
            id: "rampart1".to_string(),
            pos: Position { x: 5, y: 5 },
            hits: 0,
            max_hits: 1000,
            owner: Owner::Bot1,
            controlled_by: None,
        });

        exec.apply_tick_decay();
        assert!(exec.state.objects.is_empty());
    }

    #[test]
    fn test_spawn_placement_blocked_retry() {
        let mut exec = create_test_executor();
        let spawn_pos = Position { x: 10, y: 10 };
        exec.state.objects.push(GameObject::Spawn {
            id: "spawn1".to_string(),
            pos: spawn_pos,
            hits: 5000,
            max_hits: 5000,
            owner: Owner::Bot1,
            energy: 300,
            max_energy: 300,
            spawning: Some(crate::models::SpawningProgress {
                creep_id: "creep1".to_string(),
                need_time: 1,
                remaining_time: 1,
            }),
            next_id: "creep2".to_string(),
            directions: screeps_arena::constants::DEFAULT_SPAWN_DIRECTIONS.to_vec(),
        });
        exec.state.objects.push(GameObject::Creep {
            id: "creep1".to_string(),
            pos: spawn_pos,
            hits: 100,
            max_hits: 100,
            owner: Owner::Bot1,
            fatigue: 0,
            spawning: true,
            body: vec![crate::models::BodyPartState { part: screeps_arena::constants::Part::Move, hits: 100 }],
            store: HashMap::new(),
        });

        // Block all 8 surrounding positions with walls
        for dx in [-1i32, 0, 1] {
            for dy in [-1i32, 0, 1] {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let nx = (spawn_pos.x as i32 + dx) as usize;
                let ny = (spawn_pos.y as i32 + dy) as usize;
                exec.state.terrain[nx][ny] = Terrain::Wall;
            }
        }

        exec.apply_tick_decay();

        // Since no adjacent tile is unblocked, creep should remain at (10, 10) with spawning = true
        let creep = exec.state.objects.iter().find(|o| match o {
            GameObject::Creep { id, .. } => id == "creep1",
            _ => false,
        }).unwrap();

        if let GameObject::Creep { pos, spawning, .. } = creep {
            assert_eq!(*pos, spawn_pos);
            assert_eq!(*spawning, true);
        } else {
            panic!("Expected creep");
        }
    }

    #[test]
    fn test_creep_body_encoding_and_decoding() {
        let mut exec = create_test_executor();
        let spawn_pos = Position { x: 10, y: 10 };
        exec.state.objects.push(GameObject::Spawn {
            id: "spawn1".to_string(),
            pos: spawn_pos,
            hits: 5000,
            max_hits: 5000,
            owner: Owner::Bot1,
            energy: 1000,
            max_energy: 1000,
            spawning: None,
            next_id: "creep101".to_string(),
            directions: screeps_arena::constants::DEFAULT_SPAWN_DIRECTIONS.to_vec(),
        });

        // Encode 7 distinct body parts into packed 4-bit nibbles:
        // 1=Move, 2=Work, 3=Carry, 4=Attack, 5=RangedAttack, 6=Tough, 7=Heal
        // 0x7654321 = (7<<24) | (6<<20) | (5<<16) | (4<<12) | (3<<8) | (2<<4) | (1<<0)
        let encoded_body = 0x7654321;
        let actions = vec![QueuedAction {
            actor_id: "spawn1".to_string(),
            action: ActionId::SpawnCreep,
            target_id: None,
            arg1: 7,
            arg2: encoded_body,
        }];

        exec.resolve_actions(actions, Vec::new());

        // Find the newly spawned creep
        let creep = exec.state.objects.iter().find(|o| match o {
            GameObject::Creep { id, .. } => id == "creep101",
            _ => false,
        }).expect("Spawned creep not found in state");

        if let GameObject::Creep { body, .. } = creep {
            assert_eq!(body.len(), 7);
            assert_eq!(body[0].part, screeps_arena::constants::Part::Move);
            assert_eq!(body[1].part, screeps_arena::constants::Part::Work);
            assert_eq!(body[2].part, screeps_arena::constants::Part::Carry);
            assert_eq!(body[3].part, screeps_arena::constants::Part::Attack);
            assert_eq!(body[4].part, screeps_arena::constants::Part::RangedAttack);
            assert_eq!(body[5].part, screeps_arena::constants::Part::Tough);
            assert_eq!(body[6].part, screeps_arena::constants::Part::Heal);
        } else {
            panic!("Expected GameObject::Creep");
        }
    }

    #[test]
    fn test_creep_action_pipeline_chains_and_resource_actions() {
        let mut exec = create_test_executor();
        let creep_pos = Position { x: 10, y: 10 };
        let source_pos = Position { x: 10, y: 11 };

        // Creep with WORK & CARRY parts at (10, 10)
        exec.state.objects.push(GameObject::Creep {
            id: "creep1".to_string(),
            pos: creep_pos,
            hits: 200,
            max_hits: 200,
            owner: Owner::Bot1,
            fatigue: 0,
            spawning: false,
            body: vec![
                crate::models::BodyPartState { part: screeps_arena::constants::Part::Work, hits: 100 },
                crate::models::BodyPartState { part: screeps_arena::constants::Part::Carry, hits: 100 },
            ],
            store: HashMap::new(),
        });

        // Source at (10, 11) with 1000 energy
        exec.state.objects.push(GameObject::Source {
            id: "source1".to_string(),
            pos: source_pos,
            energy: 1000,
            max_energy: 1000,
        });

        // Queue both Harvest and Attack for creep1 (Harvest comes first in Chain 1, so Attack must be filtered out)
        let actions1 = vec![
            QueuedAction {
                actor_id: "creep1".to_string(),
                action: ActionId::Harvest,
                target_id: Some("source1".to_string()),
                arg1: 0,
                arg2: 0,
            },
            QueuedAction {
                actor_id: "creep1".to_string(),
                action: ActionId::Attack,
                target_id: Some("source1".to_string()),
                arg1: 0,
                arg2: 0,
            },
        ];

        exec.resolve_actions(actions1, Vec::new());

        // Source energy should be reduced by 2 (1 WORK part * 2) -> 998
        // Creep energy store should receive 2
        let source = exec.state.objects.iter().find(|o| match o {
            GameObject::Source { id, .. } => id == "source1",
            _ => false,
        }).unwrap();

        if let GameObject::Source { energy, .. } = source {
            assert_eq!(*energy, 998);
        } else {
            panic!("Expected Source");
        }

        let creep = exec.state.objects.iter().find(|o| match o {
            GameObject::Creep { id, .. } => id == "creep1",
            _ => false,
        }).unwrap();

        if let GameObject::Creep { store, .. } = creep {
            assert_eq!(*store.get(&screeps_arena::constants::ResourceType::Energy).unwrap_or(&0), 2);
        } else {
            panic!("Expected Creep");
        }
    }

    #[test]
    fn test_part_damage_capacity_drop_and_resource_decay() {
        let mut exec = create_test_executor();
        let creep_pos = Position { x: 10, y: 10 };

        // Creep with CARRY (index 0) + MOVE (index 1) parts, holding 50 energy
        let mut store = HashMap::new();
        store.insert(screeps_arena::constants::ResourceType::Energy, 50);

        exec.state.objects.push(GameObject::Creep {
            id: "creep1".to_string(),
            pos: creep_pos,
            hits: 200,
            max_hits: 200,
            owner: Owner::Bot1,
            fatigue: 0,
            spawning: false,
            body: vec![
                crate::models::BodyPartState { part: screeps_arena::constants::Part::Carry, hits: 100 },
                crate::models::BodyPartState { part: screeps_arena::constants::Part::Move, hits: 100 },
            ],
            store,
        });

        // Add enemy creep with 1 ATTACK part to inflict 30 damage, or queue attack directly
        exec.state.objects.push(GameObject::Creep {
            id: "enemy".to_string(),
            pos: Position { x: 10, y: 11 },
            hits: 100,
            max_hits: 100,
            owner: Owner::Bot2,
            fatigue: 0,
            spawning: false,
            body: vec![crate::models::BodyPartState { part: screeps_arena::constants::Part::Attack, hits: 100 }],
            store: HashMap::new(),
        });

        // 1. Directly apply 100 damage to test part damage distribution (disables index 0 CARRY part with 100 HP)
        for obj in &mut exec.state.objects {
            if let GameObject::Creep { id, hits, body, .. } = obj {
                if id == "creep1" {
                    let mut remaining_dmg = 100;
                    for part in body.iter_mut() {
                        if remaining_dmg == 0 {
                            break;
                        }
                        let d = part.hits.min(remaining_dmg);
                        part.hits -= d;
                        remaining_dmg -= d;
                    }
                    *hits = body.iter().map(|p| p.hits).sum();
                }
            }
        }

        // Verify index 0 CARRY part is disabled (hits = 0) and total hits is 100
        let creep = exec.state.objects.iter().find(|o| match o {
            GameObject::Creep { id, .. } => id == "creep1",
            _ => false,
        }).unwrap();

        if let GameObject::Creep { body, hits, .. } = creep {
            assert_eq!(*hits, 100);
            assert_eq!(body[0].hits, 0); // Index 0 CARRY part disabled first
            assert_eq!(body[1].hits, 100);
        } else {
            panic!("Expected Creep");
        }

        // 2. Apply tick decay: Creep loses carry capacity (0 active CARRY parts), drops 50 energy on ground, and Resource decays
        exec.apply_tick_decay();

        // Creep store should now be 0
        let creep_after = exec.state.objects.iter().find(|o| match o {
            GameObject::Creep { id, .. } => id == "creep1",
            _ => false,
        }).unwrap();

        if let GameObject::Creep { store, .. } = creep_after {
            assert_eq!(*store.get(&screeps_arena::constants::ResourceType::Energy).unwrap_or(&0), 0);
        }

        // Ground Resource object created at (10, 10) with 50 energy, minus 1 decay (ceil(50/1000) = 1) -> 49 energy
        let resource = exec.state.objects.iter().find(|o| match o {
            GameObject::Resource { .. } => true,
            _ => false,
        }).expect("Dropped Resource not found");

        if let GameObject::Resource { pos, amount, .. } = resource {
            assert_eq!(*pos, creep_pos);
            assert_eq!(*amount, 49);
        }
    }

    #[test]
    fn test_ranged_mass_attack_dropoff_and_healing() {
        let mut exec = create_test_executor();

        // Attacker creep at (10, 10) with 1 RANGED_ATTACK part (Bot1)
        exec.state.objects.push(GameObject::Creep {
            id: "attacker".to_string(),
            pos: Position { x: 10, y: 10 },
            hits: 100,
            max_hits: 100,
            owner: Owner::Bot1,
            fatigue: 0,
            spawning: false,
            body: vec![crate::models::BodyPartState { part: screeps_arena::constants::Part::RangedAttack, hits: 100 }],
            store: HashMap::new(),
        });

        // Hostile creep 1 at range 1 (10, 11) with 100 max hits (Bot2)
        exec.state.objects.push(GameObject::Creep {
            id: "enemy_r1".to_string(),
            pos: Position { x: 10, y: 11 },
            hits: 100,
            max_hits: 100,
            owner: Owner::Bot2,
            fatigue: 0,
            spawning: false,
            body: vec![crate::models::BodyPartState { part: screeps_arena::constants::Part::Move, hits: 100 }],
            store: HashMap::new(),
        });

        // Hostile creep 2 at range 2 (10, 12) with 100 max hits (Bot2)
        exec.state.objects.push(GameObject::Creep {
            id: "enemy_r2".to_string(),
            pos: Position { x: 10, y: 12 },
            hits: 100,
            max_hits: 100,
            owner: Owner::Bot2,
            fatigue: 0,
            spawning: false,
            body: vec![crate::models::BodyPartState { part: screeps_arena::constants::Part::Move, hits: 100 }],
            store: HashMap::new(),
        });

        // Hostile creep 3 at range 3 (10, 13) with 100 max hits (Bot2)
        exec.state.objects.push(GameObject::Creep {
            id: "enemy_r3".to_string(),
            pos: Position { x: 10, y: 13 },
            hits: 100,
            max_hits: 100,
            owner: Owner::Bot2,
            fatigue: 0,
            spawning: false,
            body: vec![crate::models::BodyPartState { part: screeps_arena::constants::Part::Move, hits: 100 }],
            store: HashMap::new(),
        });

        // Queue RangedMassAttack
        let actions = vec![QueuedAction {
            actor_id: "attacker".to_string(),
            action: ActionId::RangedMassAttack,
            target_id: None,
            arg1: 0,
            arg2: 0,
        }];

        exec.resolve_actions(actions, Vec::new());

        // Range 1 (enemy_r1) receives 10 damage -> 90 hits
        let r1 = exec.state.objects.iter().find(|o| match o {
            GameObject::Creep { id, .. } => id == "enemy_r1",
            _ => false,
        }).unwrap();
        if let GameObject::Creep { hits, .. } = r1 {
            assert_eq!(*hits, 90);
        }

        // Range 2 (enemy_r2) receives 4 damage -> 96 hits
        let r2 = exec.state.objects.iter().find(|o| match o {
            GameObject::Creep { id, .. } => id == "enemy_r2",
            _ => false,
        }).unwrap();
        if let GameObject::Creep { hits, .. } = r2 {
            assert_eq!(*hits, 96);
        }

        // Range 3 (enemy_r3) receives 1 damage -> 99 hits
        let r3 = exec.state.objects.iter().find(|o| match o {
            GameObject::Creep { id, .. } => id == "enemy_r3",
            _ => false,
        }).unwrap();
        if let GameObject::Creep { hits, .. } = r3 {
            assert_eq!(*hits, 99);
        }
    }

    #[test]
    fn test_simultaneous_damage_and_heal_intent_summing() {
        let mut exec = create_test_executor();
        let target_pos = Position { x: 10, y: 10 };

        // Target creep with 150 hits / 200 max_hits (Bot1)
        exec.state.objects.push(GameObject::Creep {
            id: "target".to_string(),
            pos: target_pos,
            hits: 150,
            max_hits: 200,
            owner: Owner::Bot1,
            fatigue: 0,
            spawning: false,
            body: vec![
                crate::models::BodyPartState { part: screeps_arena::constants::Part::Tough, hits: 50 },
                crate::models::BodyPartState { part: screeps_arena::constants::Part::Move, hits: 100 },
            ],
            store: HashMap::new(),
        });

        // Healer creep with 5 HEAL parts (power = 60) (Bot1)
        exec.state.objects.push(GameObject::Creep {
            id: "healer".to_string(),
            pos: Position { x: 10, y: 11 },
            hits: 500,
            max_hits: 500,
            owner: Owner::Bot1,
            fatigue: 0,
            spawning: false,
            body: vec![crate::models::BodyPartState { part: screeps_arena::constants::Part::Heal, hits: 100 }; 5],
            store: HashMap::new(),
        });

        // Attacker creep with 1 ATTACK part (power = 30) (Bot2)
        exec.state.objects.push(GameObject::Creep {
            id: "attacker".to_string(),
            pos: Position { x: 11, y: 10 },
            hits: 100,
            max_hits: 100,
            owner: Owner::Bot2,
            fatigue: 0,
            spawning: false,
            body: vec![crate::models::BodyPartState { part: screeps_arena::constants::Part::Attack, hits: 100 }],
            store: HashMap::new(),
        });

        // Queue Heal from Bot1 and Attack from Bot2 in the same tick
        let actions1 = vec![QueuedAction {
            actor_id: "healer".to_string(),
            action: ActionId::Heal,
            target_id: Some("target".to_string()),
            arg1: 0,
            arg2: 0,
        }];
        let actions2 = vec![QueuedAction {
            actor_id: "attacker".to_string(),
            action: ActionId::Attack,
            target_id: Some("target".to_string()),
            arg1: 0,
            arg2: 0,
        }];

        exec.resolve_actions(actions1, actions2);

        // Net change = 60 heal - 30 damage = +30 net heal
        // 150 hits + 30 net heal = 180 hits (capped at 200 max_hits)
        let target = exec.state.objects.iter().find(|o| match o {
            GameObject::Creep { id, .. } => id == "target",
            _ => false,
        }).unwrap();

        if let GameObject::Creep { hits, max_hits, body, .. } = target {
            assert_eq!(*hits, 180);
            assert_eq!(*max_hits, 200);
            assert_eq!(body[0].hits, 80); // Index 0 TOUGH part restored from 50 to 80
            assert_eq!(body[1].hits, 100);
        } else {
            panic!("Expected Creep");
        }
    }

    #[test]
    fn test_construction_site_build_and_completion_transformation() {
        let mut exec = create_test_executor();
        let builder_pos = Position { x: 10, y: 10 };
        let site_pos = Position { x: 10, y: 11 };

        let mut store = HashMap::new();
        store.insert(screeps_arena::constants::ResourceType::Energy, 100);

        // Builder Creep with 1 WORK part (build power = 5) and 100 energy
        exec.state.objects.push(GameObject::Creep {
            id: "builder".to_string(),
            pos: builder_pos,
            hits: 100,
            max_hits: 100,
            owner: Owner::Bot1,
            fatigue: 0,
            spawning: false,
            body: vec![crate::models::BodyPartState { part: screeps_arena::constants::Part::Work, hits: 100 }],
            store,
        });

        // Extension ConstructionSite at (10, 11) with 95 progress / 100 progress_total
        exec.state.objects.push(GameObject::ConstructionSite {
            id: "site1".to_string(),
            pos: site_pos,
            owner: Owner::Bot1,
            progress: 95,
            progress_total: 100,
            structure_type: crate::models::StructureType::Extension,
        });

        // Queue Build action
        let actions = vec![QueuedAction {
            actor_id: "builder".to_string(),
            action: ActionId::Build,
            target_id: Some("site1".to_string()),
            arg1: 0,
            arg2: 0,
        }];

        exec.resolve_actions(actions, Vec::new());

        // Site progress should reach 100 (95 + 5 = 100)
        // Builder energy deducted by 5 (100 -> 95)
        let site = exec.state.objects.iter().find(|o| match o {
            GameObject::ConstructionSite { id, .. } => id == "site1",
            _ => false,
        }).expect("Construction site not found");

        if let GameObject::ConstructionSite { progress, .. } = site {
            assert_eq!(*progress, 100);
        }

        // Apply tick decay: completed site is transformed into an Extension
        exec.apply_tick_decay();

        let site_exists = exec.state.objects.iter().any(|o| match o {
            GameObject::ConstructionSite { id, .. } => id == "site1",
            _ => false,
        });
        assert!(!site_exists, "ConstructionSite should be removed");

        let extension = exec.state.objects.iter().find(|o| match o {
            GameObject::Extension { id, .. } => id == "site1",
            _ => false,
        }).expect("Completed Extension structure not found");

        if let GameObject::Extension { pos, hits, max_hits, owner, .. } = extension {
            assert_eq!(*pos, site_pos);
            assert_eq!(*hits, 500);
            assert_eq!(*max_hits, 500);
            assert_eq!(*owner, Owner::Bot1);
        }
    }

    #[test]
    fn test_spawning_lifecycle_and_completion_timing() {
        let mut exec = create_test_executor();
        let spawn_pos = Position { x: 49, y: 4 };

        exec.state.objects.push(GameObject::Spawn {
            id: "spawn121".to_string(),
            pos: spawn_pos,
            hits: 5000,
            max_hits: 5000,
            owner: Owner::Bot1,
            energy: 1000,
            max_energy: 1000,
            spawning: None,
            next_id: "creep335".to_string(),
            directions: screeps_arena::constants::DEFAULT_SPAWN_DIRECTIONS.to_vec(),
        });

        // 2 body parts (Move + Attack) = 6 ticks needTime
        let encoded_body = 0x41; // Move=1, Attack=4
        let actions = vec![QueuedAction {
            actor_id: "spawn121".to_string(),
            action: ActionId::SpawnCreep,
            target_id: None,
            arg1: 2,
            arg2: encoded_body,
        }];

        // Tick 1: Action resolved, spawn progress initialized with needTime=6, spawnTime=7
        exec.resolve_actions(actions, Vec::new());
        exec.apply_tick_decay();

        let spawn_obj = exec.state.objects.iter().find(|o| match o {
            GameObject::Spawn { id, .. } => id == "spawn121",
            _ => false,
        }).unwrap();

        if let GameObject::Spawn { spawning, .. } = spawn_obj {
            let sp = spawning.as_ref().unwrap();
            assert_eq!(sp.need_time, 6);
            assert_eq!(sp.remaining_time, 7);
        }

        // Ticks 2 through 5: Spawning remains active
        for _ in 2..=5 {
            exec.state.tick += 1;
            exec.apply_tick_decay();
            let creep = exec.state.objects.iter().find(|o| match o {
                GameObject::Creep { id, .. } => id == "creep335",
                _ => false,
            }).unwrap();
            if let GameObject::Creep { spawning, .. } = creep {
                assert!(*spawning, "Creep should remain spawning during ticks 2-5");
            }
        }

        // Tick 6: Spawning completes at end of tick decay phase
        exec.state.tick += 1; // tick = 6
        exec.apply_tick_decay();

        let spawn_after = exec.state.objects.iter().find(|o| match o {
            GameObject::Spawn { id, .. } => id == "spawn121",
            _ => false,
        }).unwrap();
        if let GameObject::Spawn { spawning, .. } = spawn_after {
            assert!(spawning.is_none(), "Spawn spawning state should be None at end of tick 6");
        }

        let creep_after = exec.state.objects.iter().find(|o| match o {
            GameObject::Creep { id, .. } => id == "creep335",
            _ => false,
        }).unwrap();
        if let GameObject::Creep { spawning, pos, .. } = creep_after {
            assert!(!*spawning, "Creep spawning state should be false at end of tick 6");
            assert_eq!(*pos, Position { x: 49, y: 3 }, "Creep should be placed at adjacent direction tile (49, 3)");
        }
    }

    #[test]
    fn test_spawning_fatigue_and_old_fatigue_serialization() {
        let mut exec = create_test_executor();
        let spawn_pos = Position { x: 49, y: 4 };

        exec.state.objects.push(GameObject::Spawn {
            id: "spawn121".to_string(),
            pos: spawn_pos,
            hits: 5000,
            max_hits: 5000,
            owner: Owner::Bot1,
            energy: 1000,
            max_energy: 1000,
            spawning: None,
            next_id: "creep335".to_string(),
            directions: screeps_arena::constants::DEFAULT_SPAWN_DIRECTIONS.to_vec(),
        });

        // 2 body parts (Move + Attack)
        let encoded_body = 0x41;
        let actions = vec![QueuedAction {
            actor_id: "spawn121".to_string(),
            action: ActionId::SpawnCreep,
            target_id: None,
            arg1: 2,
            arg2: encoded_body,
        }];

        exec.resolve_actions(actions, Vec::new());
        exec.apply_tick_decay();

        // While spawning, serialized JSON must contain negative _fatigue (-2) and no _oldFatigue (null)
        let frame_tick1 = exec.state.to_replay_json(None, &exec.last_action_logs);
        let creep_json = frame_tick1["objects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o.get("_id") == Some(&serde_json::json!("creep335")))
            .unwrap();

        assert_eq!(creep_json["_fatigue"], serde_json::json!(-2));
        assert!(creep_json.get("_oldFatigue").map_or(true, |v| v.is_null()));
    }
}
