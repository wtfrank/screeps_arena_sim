use std::collections::HashMap;
use std::time::Duration;
use anyhow::{Context, Result};

use crate::models::{GameObject, Position, Owner, Terrain, GameState, Ruleset, WinCondition};
use crate::driver::{BotDriver, QueuedAction};
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
}

impl RunExecutor {
    pub fn new(
        initial_state: GameState,
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

        // 2. Run active (non-crashed) bots in parallel threads
        let timeout_b1 = if self.debug_b1 { Duration::from_secs(3600) } else { Duration::from_millis(self.rules.cpu_time_limit as u64) };
        let timeout_b2 = if self.debug_b2 { Duration::from_secs(3600) } else { Duration::from_millis(self.rules.cpu_time_limit as u64) };
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
                        };
                        d2.tick(msg, timeout_b2)
                    } else {
                        Ok(Vec::new())
                    }
                } else {
                    Ok(Vec::new())
                }
            }
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
            GameObject::Spawn { owner: Owner::Bot1, .. } => true,
            _ => false,
        });
        let bot2_spawn_exists = self.state.objects.iter().any(|o| match o {
            GameObject::Spawn { owner: Owner::Bot2, .. } => true,
            _ => false,
        });

        match self.rules.win_condition {
            WinCondition::DestroyEnemySpawn => {
                if bot1_spawn_exists && !bot2_spawn_exists {
                    SimulationResult::Bot1Win { reason: "Enemy spawn destroyed".to_string() }
                } else if !bot1_spawn_exists && bot2_spawn_exists {
                    SimulationResult::Bot2Win { reason: "Spawn destroyed".to_string() }
                } else if limit_reached {
                    SimulationResult::Draw { reason: "Tick limit reached".to_string() }
                } else if self.bot1_crashed && self.bot2_crashed {
                    SimulationResult::Draw { reason: "Both bots crashed".to_string() }
                } else {
                    SimulationResult::Draw { reason: "Both spawns destroyed simultaneously".to_string() }
                }
            }
            WinCondition::Survival => {
                if limit_reached {
                    SimulationResult::Draw { reason: "Tick limit reached".to_string() }
                } else if !bot1_spawn_exists && !bot2_spawn_exists {
                    SimulationResult::Draw { reason: "No survivors".to_string() }
                } else if bot1_spawn_exists {
                    SimulationResult::Bot1Win { reason: "Survived".to_string() }
                } else {
                    SimulationResult::Bot2Win { reason: "Survived".to_string() }
                }
            }
            WinCondition::HighestScore => {
                SimulationResult::Draw { reason: "Score conditions unresolved".to_string() }
            }
        }
    }

    fn check_win_conditions_active(&self) -> Option<SimulationResult> {
        let bot1_spawn_exists = self.state.objects.iter().any(|o| match o {
            GameObject::Spawn { owner: Owner::Bot1, .. } => true,
            _ => false,
        });
        let bot2_spawn_exists = self.state.objects.iter().any(|o| match o {
            GameObject::Spawn { owner: Owner::Bot2, .. } => true,
            _ => false,
        });

        if !bot1_spawn_exists || !bot2_spawn_exists {
            return Some(self.check_win_condition(false));
        }
        None
    }

    fn resolve_actions(&mut self, actions1: Vec<QueuedAction>, actions2: Vec<QueuedAction>) {
        // Resolve movement intents
        let mut move_intents = HashMap::new();
        for act in actions1.iter().filter(|a| a.action == ActionId::Move) {
            move_intents.insert(act.actor_id.clone(), (act.arg1 as u8, Owner::Bot1));
        }
        for act in actions2.iter().filter(|a| a.action == ActionId::Move) {
            move_intents.insert(act.actor_id.clone(), (act.arg1 as u8, Owner::Bot2));
        }

        // Apply movement changes (simplified: direct write, bounce on overlap)
        let mut new_positions = HashMap::new();
        for obj in &self.state.objects {
            if let GameObject::Creep { id, pos, .. } = obj {
                if let Some(&(direction, _)) = move_intents.get(id) {
                    let mut next_pos = *pos;
                    match direction {
                        1 => { next_pos.y = next_pos.y.saturating_sub(1); } // Top
                        2 => { next_pos.x = next_pos.x.saturating_add(1); next_pos.y = next_pos.y.saturating_sub(1); } // TopRight
                        3 => { next_pos.x = next_pos.x.saturating_add(1); } // Right
                        4 => { next_pos.x = next_pos.x.saturating_add(1); next_pos.y = next_pos.y.saturating_add(1); } // BottomRight
                        5 => { next_pos.y = next_pos.y.saturating_add(1); } // Bottom
                        6 => { next_pos.x = next_pos.x.saturating_sub(1); next_pos.y = next_pos.y.saturating_add(1); } // BottomLeft
                        7 => { next_pos.x = next_pos.x.saturating_sub(1); } // Left
                        8 => { next_pos.x = next_pos.x.saturating_sub(1); next_pos.y = next_pos.y.saturating_sub(1); } // TopLeft
                        _ => {}
                    }
                    // Prevent leaving bounds
                    if next_pos.x < self.state.width && next_pos.y < self.state.height {
                        new_positions.insert(id.clone(), next_pos);
                    }
                }
            }
        }

        // Collect positions of non-moving creeps to avoid double borrows inside the mutation loop
        let non_moving_occupied: std::collections::HashSet<Position> = self.state.objects.iter().filter_map(|o| match o {
            GameObject::Creep { id, pos, .. } if !new_positions.contains_key(id) => Some(*pos),
            _ => None
        }).collect();

        // Apply updates back to the state objects
        for obj in &mut self.state.objects {
            if let GameObject::Creep { id, pos, fatigue, .. } = obj {
                if let Some(next_pos) = new_positions.get(id) {
                    let target_occupied = non_moving_occupied.contains(next_pos);

                    if !target_occupied {
                        *pos = *next_pos;
                        *fatigue = fatigue.saturating_add(2);
                    }
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

        // Remove destroyed units
        self.state.objects.retain(|o| match o {
            GameObject::Creep { hits, .. } => *hits > 0,
            GameObject::Spawn { hits, .. } => *hits > 0,
            GameObject::Tower { hits, .. } => *hits > 0,
            _ => true,
        });
    }

    fn apply_tick_decay(&mut self) {
        for obj in &mut self.state.objects {
            if let GameObject::Creep { fatigue, .. } = obj {
                *fatigue = fatigue.saturating_sub(2);
            }
        }
    }

    // Helper functions to construct mock_screeps_arena structures with proper `my()` logic:

    fn get_mock_creeps(&self, is_bot1: bool) -> Vec<screeps_arena::objects::Creep> {
        self.state.objects.iter().filter_map(|o| match o {
            GameObject::Creep { id, pos, hits, max_hits, owner, fatigue } => {
                let my = if is_bot1 { *owner == Owner::Bot1 } else { *owner == Owner::Bot2 };
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
                })
            }
            _ => None,
        }).collect()
    }

    fn get_mock_spawns(&self, is_bot1: bool) -> Vec<screeps_arena::objects::StructureSpawn> {
        self.state.objects.iter().filter_map(|o| match o {
            GameObject::Spawn { id, pos, hits, max_hits, owner, energy, max_energy } => {
                let my = if is_bot1 { *owner == Owner::Bot1 } else { *owner == Owner::Bot2 };
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
                })
            }
            _ => None,
        }).collect()
    }

    fn get_mock_towers(&self, is_bot1: bool) -> Vec<screeps_arena::objects::StructureTower> {
        self.state.objects.iter().filter_map(|o| match o {
            GameObject::Tower { id, pos, hits, max_hits, owner, energy, max_energy } => {
                let my = if is_bot1 { *owner == Owner::Bot1 } else { *owner == Owner::Bot2 };
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
        }).collect()
    }

    fn get_mock_extensions(&self, is_bot1: bool) -> Vec<screeps_arena::objects::StructureExtension> {
        self.state.objects.iter().filter_map(|o| match o {
            GameObject::Extension { id, pos, hits, max_hits, owner, energy, max_energy } => {
                let my = if is_bot1 { *owner == Owner::Bot1 } else { *owner == Owner::Bot2 };
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
        }).collect()
    }

    fn get_mock_ramparts(&self, is_bot1: bool) -> Vec<screeps_arena::objects::StructureRampart> {
        self.state.objects.iter().filter_map(|o| match o {
            GameObject::Rampart { id, pos, hits, max_hits, owner } => {
                let my = if is_bot1 { *owner == Owner::Bot1 } else { *owner == Owner::Bot2 };
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
        }).collect()
    }

    fn get_mock_containers(&self) -> Vec<screeps_arena::objects::StructureContainer> {
        self.state.objects.iter().filter_map(|o| match o {
            GameObject::Container { id, pos, hits, max_hits, energy, max_energy } => {
                Some(screeps_arena::objects::StructureContainer {
                    base: screeps_arena::objects::GameObject {
                        id: id.clone(),
                        x: pos.x,
                        y: pos.y,
                    },
                    hits: *hits,
                    hits_max: *max_hits,
                    store: *energy,
                    store_max: *max_energy,
                })
            }
            _ => None,
        }).collect()
    }

    fn get_mock_roads(&self) -> Vec<screeps_arena::objects::StructureRoad> {
        self.state.objects.iter().filter_map(|o| match o {
            GameObject::Road { id, pos, hits, max_hits } => {
                Some(screeps_arena::objects::StructureRoad {
                    base: screeps_arena::objects::GameObject {
                        id: id.clone(),
                        x: pos.x,
                        y: pos.y,
                    },
                    hits: *hits,
                    hits_max: *max_hits,
                })
            }
            _ => None,
        }).collect()
    }

    fn get_mock_walls(&self) -> Vec<screeps_arena::objects::StructureWall> {
        self.state.objects.iter().filter_map(|o| match o {
            GameObject::Wall { id, pos, hits, max_hits } => {
                Some(screeps_arena::objects::StructureWall {
                    base: screeps_arena::objects::GameObject {
                        id: id.clone(),
                        x: pos.x,
                        y: pos.y,
                    },
                    hits: *hits,
                    hits_max: *max_hits,
                })
            }
            _ => None,
        }).collect()
    }

    fn get_mock_resources(&self) -> Vec<screeps_arena::objects::Resource> {
        self.state.objects.iter().filter_map(|o| match o {
            GameObject::Resource { id, pos, amount, resource_type } => {
                Some(screeps_arena::objects::Resource {
                    base: screeps_arena::objects::GameObject {
                        id: id.clone(),
                        x: pos.x,
                        y: pos.y,
                    },
                    amount: *amount,
                    resource_type: resource_type.clone(),
                })
            }
            _ => None,
        }).collect()
    }

    fn get_mock_sources(&self) -> Vec<screeps_arena::objects::Source> {
        self.state.objects.iter().filter_map(|o| match o {
            GameObject::Source { id, pos, energy, max_energy } => {
                Some(screeps_arena::objects::Source {
                    base: screeps_arena::objects::GameObject {
                        id: id.clone(),
                        x: pos.x,
                        y: pos.y,
                    },
                    energy: *energy,
                    energy_max: *max_energy,
                })
            }
            _ => None,
        }).collect()
    }

    fn get_mock_flags(&self, is_bot1: bool) -> Vec<screeps_arena::objects::Flag> {
        self.state.objects.iter().filter_map(|o| match o {
            GameObject::Flag { id, pos, owner } => {
                let my = if is_bot1 { *owner == Owner::Bot1 } else { *owner == Owner::Bot2 };
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
        }).collect()
    }

    fn get_mock_score_collectors(&self, is_bot1: bool) -> Vec<screeps_arena::objects::ScoreCollector> {
        self.state.objects.iter().filter_map(|o| match o {
            GameObject::ScoreCollector { id, pos, owner } => {
                let my = if is_bot1 { *owner == Owner::Bot1 } else { *owner == Owner::Bot2 };
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
        }).collect()
    }

    fn get_mock_bonus_flags(&self, is_bot1: bool) -> Vec<screeps_arena::objects::BonusFlag> {
        self.state.objects.iter().filter_map(|o| match o {
            GameObject::BonusFlag { id, pos, owner } => {
                let my = if is_bot1 { *owner == Owner::Bot1 } else { *owner == Owner::Bot2 };
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
        }).collect()
    }

    fn get_mock_area_effects(&self) -> Vec<screeps_arena::objects::AreaEffect> {
        self.state.objects.iter().filter_map(|o| match o {
            GameObject::AreaEffect { id, pos, effect_type } => {
                Some(screeps_arena::objects::AreaEffect {
                    base: screeps_arena::objects::GameObject {
                        id: id.clone(),
                        x: pos.x,
                        y: pos.y,
                    },
                    effect_type: effect_type.clone(),
                })
            }
            _ => None,
        }).collect()
    }

    fn get_mock_construction_sites(&self, is_bot1: bool) -> Vec<screeps_arena::objects::ConstructionSite> {
        self.state.objects.iter().filter_map(|o| match o {
            GameObject::ConstructionSite { id, pos, owner, progress, progress_total } => {
                let my = if is_bot1 { *owner == Owner::Bot1 } else { *owner == Owner::Bot2 };
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
        }).collect()
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

        let res1 = handle1.join().unwrap_or_else(|_| Err(anyhow::anyhow!("Bot A thread join failed")));
        let res2 = handle2.join().unwrap_or_else(|_| Err(anyhow::anyhow!("Bot B thread join failed")));

        (res1, res2)
    })
}
