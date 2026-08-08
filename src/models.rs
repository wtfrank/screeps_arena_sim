use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Position {
    pub x: u8,
    pub y: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Owner {
    Neutral,
    Bot1,
    Bot2,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawningProgress {
    pub creep_id: String,
    pub need_time: u32,
    pub remaining_time: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BodyPartState {
    pub part: screeps_arena::constants::Part,
    pub hits: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StructureType {
    Spawn,
    Extension,
    Tower,
    Container,
    Rampart,
    Road,
    Wall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GameObject {
    Creep {
        id: String,
        pos: Position,
        hits: u32,
        max_hits: u32,
        owner: Owner,
        fatigue: u8,
        spawning: bool,
        body: Vec<BodyPartState>,
        store: std::collections::HashMap<screeps_arena::constants::ResourceType, u32>,
    },
    Spawn {
        id: String,
        pos: Position,
        hits: u32,
        max_hits: u32,
        owner: Owner,
        energy: u32,
        max_energy: u32,
        spawning: Option<SpawningProgress>,
        next_id: String,
        directions: Vec<screeps_arena::constants::Direction>,
    },
    Tower {
        id: String,
        pos: Position,
        hits: u32,
        max_hits: u32,
        owner: Owner,
        energy: u32,
        max_energy: u32,
    },
    Extension {
        id: String,
        pos: Position,
        hits: u32,
        max_hits: u32,
        owner: Owner,
        energy: u32,
        max_energy: u32,
    },
    Rampart {
        id: String,
        pos: Position,
        hits: u32,
        max_hits: u32,
        owner: Owner,
        controlled_by: Option<String>,
    },
    Container {
        id: String,
        pos: Position,
        hits: u32,
        max_hits: u32,
        energy: u32,
        max_energy: u32,
    },
    Road {
        id: String,
        pos: Position,
        hits: u32,
        max_hits: u32,
    },
    Wall {
        id: String,
        pos: Position,
        hits: u32,
        max_hits: u32,
    },
    ConstructionSite {
        id: String,
        pos: Position,
        owner: Owner,
        progress: u32,
        progress_total: u32,
        structure_type: StructureType,
    },
    Resource {
        id: String,
        pos: Position,
        amount: u32,
        resource_type: String,
    },
    Source {
        id: String,
        pos: Position,
        energy: u32,
        max_energy: u32,
    },
    Flag {
        id: String,
        pos: Position,
        owner: Owner,
    },
    ScoreCollector {
        id: String,
        pos: Position,
        owner: Owner,
    },
    BonusFlag {
        id: String,
        pos: Position,
        owner: Owner,
    },
    AreaEffect {
        id: String,
        pos: Position,
        effect_type: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Terrain {
    Plain,
    Wall,
    Swamp,
}

impl Terrain {
    pub fn parse_string(raw: &str, width: usize, height: usize) -> Vec<Vec<Terrain>> {
        let mut grid = vec![vec![Terrain::Plain; height]; width];
        let bytes = raw.as_bytes();
        for y in 0..height {
            for x in 0..width {
                let idx = y * width + x;
                if idx < bytes.len() {
                    grid[x][y] = match bytes[idx] {
                        b'1' | b'3' => Terrain::Wall,
                        b'2' => Terrain::Swamp,
                        _ => Terrain::Plain,
                    };
                }
            }
        }
        grid
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapLayout {
    pub name: String,
    pub width: u8,
    pub height: u8,
    pub terrain: Vec<Vec<Terrain>>,
    pub initial_objects: Vec<GameObject>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameState {
    pub tick: u32,
    pub width: u8,
    pub height: u8,
    pub objects: Vec<GameObject>,
    pub terrain: Vec<Vec<Terrain>>,
}

impl GameState {
    pub fn to_replay_json(
        &self,
        users_map: Option<serde_json::Value>,
        action_logs: &std::collections::HashMap<String, serde_json::Value>,
    ) -> serde_json::Value {
        self.to_replay_json_with_old_fatigue(users_map, action_logs, &std::collections::HashMap::new())
    }

    pub fn to_replay_json_with_old_fatigue(
        &self,
        users_map: Option<serde_json::Value>,
        action_logs: &std::collections::HashMap<String, serde_json::Value>,
        old_fatigue_map: &std::collections::HashMap<String, u8>,
    ) -> serde_json::Value {
        let mut obj_list = Vec::new();

        for obj in &self.objects {
            let mut map = serde_json::Map::new();

            // Format id as number if parseable, or string
            let (id_val, type_str, proto_str, pos, owner_opt) = match obj {
                GameObject::Creep { id, pos, hits, max_hits, owner, fatigue, spawning, body, store } => {
                    map.insert("hits".to_string(), serde_json::json!(hits));
                    map.insert("hitsMax".to_string(), serde_json::json!(max_hits));
                    map.insert("spawning".to_string(), serde_json::json!(spawning));
                    map.insert("fatigue".to_string(), serde_json::json!(fatigue));

                    let body_json: Vec<serde_json::Value> = body
                        .iter()
                        .map(|b| {
                            let type_s = match b.part {
                                screeps_arena::constants::Part::Move => "move",
                                screeps_arena::constants::Part::Work => "work",
                                screeps_arena::constants::Part::Carry => "carry",
                                screeps_arena::constants::Part::Attack => "attack",
                                screeps_arena::constants::Part::RangedAttack => "ranged_attack",
                                screeps_arena::constants::Part::Tough => "tough",
                                screeps_arena::constants::Part::Heal => "heal",
                            };
                            serde_json::json!({
                                "type": type_s,
                                "hits": b.hits
                            })
                        })
                        .collect();
                    map.insert("body".to_string(), serde_json::json!(body_json));

                    let carry_parts = body.iter().filter(|b| b.part == screeps_arena::constants::Part::Carry).count();
                    let move_parts = body.iter().filter(|b| b.part == screeps_arena::constants::Part::Move).count();

                    if *spawning {
                        map.insert("_fatigue".to_string(), serde_json::json!(-(move_parts as i32 * 2)));
                    } else if let Some(old_f) = old_fatigue_map.get(id) {
                        map.insert("_fatigue".to_string(), serde_json::json!(fatigue));
                        map.insert("_oldFatigue".to_string(), serde_json::json!(old_f));
                    } else {
                        // Creep just finished spawning on this tick
                        map.insert("_fatigue".to_string(), serde_json::json!(-(move_parts as i32 * 2)));
                    }

                    let mut store_map = serde_json::Map::new();
                    for (res, amt) in store {
                        let res_name = match res {
                            screeps_arena::constants::ResourceType::Energy => "energy",
                            screeps_arena::constants::ResourceType::Score => "score",
                            screeps_arena::constants::ResourceType::ScoreX => "score_x",
                            screeps_arena::constants::ResourceType::ScoreY => "score_y",
                            screeps_arena::constants::ResourceType::ScoreZ => "score_z",
                        };
                        store_map.insert(res_name.to_string(), serde_json::json!(amt));
                    }
                    if carry_parts > 0 && !store_map.contains_key("energy") {
                        store_map.insert("energy".to_string(), serde_json::json!(0));
                    }
                    map.insert("store".to_string(), serde_json::Value::Object(store_map));

                    map.insert("storeCapacity".to_string(), serde_json::json!(carry_parts * 50));
                    map.insert("effects".to_string(), serde_json::json!([]));

                    if let Some(act_log) = action_logs.get(id) {
                        map.insert("actionLog".to_string(), act_log.clone());
                    } else if !*spawning || self.tick > 1 {
                        map.insert("actionLog".to_string(), serde_json::json!({}));
                    }

                    (id, "creep", "Creep", pos, Some(owner))
                }
                GameObject::Spawn { id, pos, hits, max_hits, owner, energy, max_energy, spawning, .. } => {
                    map.insert("hits".to_string(), serde_json::json!(hits));
                    map.insert("hitsMax".to_string(), serde_json::json!(max_hits));

                    let mut store_map = serde_json::Map::new();
                    store_map.insert("energy".to_string(), serde_json::json!(energy));
                    map.insert("store".to_string(), serde_json::Value::Object(store_map));

                    map.insert("storeCapacityResource".to_string(), serde_json::json!({ "energy": max_energy }));

                    if let Some(sp) = spawning {
                        let sp_id_val = sp.creep_id.parse::<u64>().map(serde_json::Value::from).unwrap_or_else(|_| serde_json::json!(sp.creep_id));
                        map.insert(
                            "spawning".to_string(),
                            serde_json::json!({
                                "id": sp_id_val,
                                "needTime": sp.need_time,
                                "spawnTime": sp.remaining_time
                            }),
                        );
                    } else {
                        map.insert("spawning".to_string(), serde_json::Value::Null);
                    }
                    map.insert("origin".to_string(), serde_json::json!(true));
                    map.insert("actionLog".to_string(), serde_json::json!({}));

                    (id, "spawn", "StructureSpawn", pos, Some(owner))
                }
                GameObject::Extension { id, pos, hits, max_hits, owner, energy, max_energy } => {
                    map.insert("hits".to_string(), serde_json::json!(hits));
                    map.insert("hitsMax".to_string(), serde_json::json!(max_hits));
                    map.insert("store".to_string(), serde_json::json!({ "energy": energy }));
                    map.insert("storeCapacityResource".to_string(), serde_json::json!({ "energy": max_energy }));

                    (id, "extension", "StructureExtension", pos, Some(owner))
                }
                GameObject::Tower { id, pos, hits, max_hits, owner, energy, max_energy } => {
                    map.insert("hits".to_string(), serde_json::json!(hits));
                    map.insert("hitsMax".to_string(), serde_json::json!(max_hits));
                    map.insert("store".to_string(), serde_json::json!({ "energy": energy }));
                    map.insert("storeCapacityResource".to_string(), serde_json::json!({ "energy": max_energy }));
                    map.insert("actionLog".to_string(), serde_json::json!({}));

                    (id, "tower", "StructureTower", pos, Some(owner))
                }
                GameObject::Rampart { id, pos, hits, max_hits, owner, controlled_by } => {
                    map.insert("hits".to_string(), serde_json::json!(hits));
                    map.insert("hitsMax".to_string(), serde_json::json!(max_hits));
                    if let Some(cb) = controlled_by {
                        let parsed_cb = cb.parse::<u64>().map(serde_json::Value::from).unwrap_or_else(|_| serde_json::json!(cb));
                        map.insert("controlledBy".to_string(), parsed_cb);
                    }

                    (id, "rampart", "StructureRampart", pos, Some(owner))
                }
                GameObject::Container { id, pos, hits, max_hits, energy, max_energy } => {
                    map.insert("hits".to_string(), serde_json::json!(hits));
                    map.insert("hitsMax".to_string(), serde_json::json!(max_hits));
                    map.insert("store".to_string(), serde_json::json!({ "energy": energy }));
                    map.insert("storeCapacityResource".to_string(), serde_json::json!({ "energy": max_energy }));

                    (id, "container", "StructureContainer", pos, None)
                }
                GameObject::Road { id, pos, hits, max_hits } => {
                    map.insert("hits".to_string(), serde_json::json!(hits));
                    map.insert("hitsMax".to_string(), serde_json::json!(max_hits));

                    (id, "road", "StructureRoad", pos, None)
                }
                GameObject::Wall { id, pos, hits, max_hits } => {
                    map.insert("hits".to_string(), serde_json::json!(hits));
                    map.insert("hitsMax".to_string(), serde_json::json!(max_hits));

                    (id, "constructedWall", "StructureWall", pos, None)
                }
                GameObject::ConstructionSite { id, pos, owner, progress, progress_total, structure_type } => {
                    map.insert("progress".to_string(), serde_json::json!(progress));
                    map.insert("progressTotal".to_string(), serde_json::json!(progress_total));
                    let proto_name = match structure_type {
                        StructureType::Spawn => "StructureSpawn",
                        StructureType::Extension => "StructureExtension",
                        StructureType::Tower => "StructureTower",
                        StructureType::Container => "StructureContainer",
                        StructureType::Rampart => "StructureRampart",
                        StructureType::Road => "StructureRoad",
                        StructureType::Wall => "StructureWall",
                    };
                    map.insert("structureType".to_string(), serde_json::json!(proto_name));

                    (id, "constructionSite", "ConstructionSite", pos, Some(owner))
                }
                GameObject::Resource { id, pos, amount, resource_type } => {
                    map.insert("amount".to_string(), serde_json::json!(amount));
                    map.insert("resourceType".to_string(), serde_json::json!(resource_type.to_lowercase()));

                    (id, "resource", "Resource", pos, None)
                }
                GameObject::Source { id, pos, energy, max_energy } => {
                    map.insert("energy".to_string(), serde_json::json!(energy));
                    map.insert("energyCapacity".to_string(), serde_json::json!(max_energy));

                    (id, "source", "Source", pos, None)
                }
                GameObject::Flag { id, pos, owner } => {
                    (id, "flag", "Flag", pos, Some(owner))
                }
                GameObject::ScoreCollector { id, pos, owner } => {
                    (id, "scoreCollector", "ScoreCollector", pos, Some(owner))
                }
                GameObject::BonusFlag { id, pos, owner } => {
                    (id, "bonusFlag", "BonusFlag", pos, Some(owner))
                }
                GameObject::AreaEffect { id, pos, effect_type } => {
                    map.insert("effectType".to_string(), serde_json::json!(effect_type));

                    (id, "areaEffect", "AreaEffect", pos, None)
                }
            };

            let parsed_id = id_val.parse::<u64>().map(serde_json::Value::from).unwrap_or_else(|_| serde_json::json!(id_val));
            map.insert("_id".to_string(), parsed_id);
            map.insert("type".to_string(), serde_json::json!(type_str));
            map.insert("prototypeName".to_string(), serde_json::json!(proto_str));
            map.insert("x".to_string(), serde_json::json!(pos.x));
            map.insert("y".to_string(), serde_json::json!(pos.y));

            if let Some(owner) = owner_opt {
                match owner {
                    Owner::Bot1 => {
                        map.insert("user".to_string(), serde_json::json!("player1"));
                    }
                    Owner::Bot2 => {
                        map.insert("user".to_string(), serde_json::json!("player2"));
                    }
                    Owner::Neutral => {}
                }
            }

            obj_list.push(serde_json::Value::Object(map));
        }

        let mut tick_map = serde_json::Map::new();
        tick_map.insert("gameTime".to_string(), serde_json::json!(self.tick));
        if let Some(users) = users_map {
            tick_map.insert("users".to_string(), users);
        }
        tick_map.insert("objects".to_string(), serde_json::Value::Array(obj_list));

        serde_json::Value::Object(tick_map)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ruleset {
    pub tick_limit: u32,
    pub cpu_time_limit: u32,
    pub win_condition: WinCondition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WinCondition {
    DestroyEnemySpawn,
    HighestScore,
    Survival,
}
