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
        body: Vec<screeps_arena::constants::Part>,
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
                        b'1' => Terrain::Wall,
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
