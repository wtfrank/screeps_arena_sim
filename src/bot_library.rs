use std::path::{Path, PathBuf};
use std::fs;
use std::collections::HashMap;
use anyhow::{Context, Result};
use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct KnownArena {
    pub arena_id: String,
    pub name: String,
    pub folder_name: String,
    pub advanced: bool,
}

pub fn get_known_arenas() -> Vec<KnownArena> {
    vec![
        KnownArena {
            arena_id: "69cfe6fcece2ae9f75da12d1".to_string(),
            name: "Spawn Strike 3".to_string(),
            folder_name: "season3-spawn_strike".to_string(),
            advanced: false,
        },
        KnownArena {
            arena_id: "69cfe700ece2ae9f75da12d2".to_string(),
            name: "Spawn Strike Advanced 3".to_string(),
            folder_name: "season3-spawn_strike_advanced".to_string(),
            advanced: true,
        },
        KnownArena {
            arena_id: "69cfe704ece2ae9f75da12d3".to_string(),
            name: "Power Split 3".to_string(),
            folder_name: "season3-power_split".to_string(),
            advanced: false,
        },
        KnownArena {
            arena_id: "69cfe708ece2ae9f75da12d4".to_string(),
            name: "Power Split Advanced 3".to_string(),
            folder_name: "season3-power_split_advanced".to_string(),
            advanced: true,
        },
        KnownArena {
            arena_id: "69cfe70cece2ae9f75da12d5".to_string(),
            name: "Escort Run 3".to_string(),
            folder_name: "season3-escort_run".to_string(),
            advanced: false,
        },
        KnownArena {
            arena_id: "69cfe710ece2ae9f75da12d6".to_string(),
            name: "Escort Run Advanced 3".to_string(),
            folder_name: "season3-escort_run_advanced".to_string(),
            advanced: true,
        },
    ]
}

/// Attempts to load arena layout terrain data from local files or protocol JSONs for a given arena_id.
/// Selects a layout at random if multiple layout files exist for the arena.
/// Returns an error if no valid layout is available.
pub fn load_arena_terrain(library_dir: &Path, arena_id: &str, width: u8, height: u8) -> Result<Vec<Vec<crate::models::Terrain>>> {
    let width_u = width as usize;
    let height_u = height as usize;
    let mut available_layouts = Vec::new();

    // 1. Scan <library_dir>/layouts/ directory for layouts matching <arena_id>*.json
    let layouts_dir = library_dir.join("layouts");
    if layouts_dir.exists() && layouts_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&layouts_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if filename.starts_with(arena_id) && filename.ends_with(".json") {
                    available_layouts.push(path);
                }
            }
        }
    }

    // 2. Check direct layout file <library_dir>/layouts/<arena_id>.json or fallback protocol terrain.json
    let fallback_paths = vec![
        layouts_dir.join(format!("{}.json", arena_id)),
        layouts_dir.join("terrain.json"),
        PathBuf::from("/home/ff/projects/screeps_arena/screeps_arena_protocol/terrain.json"),
    ];

    for path in fallback_paths {
        if path.exists() && !available_layouts.contains(&path) {
            available_layouts.push(path);
        }
    }

    let mut valid_grids = Vec::new();

    for path in available_layouts {
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                // Check if json matches this arena (if arena field is present, verify match)
                if let Some(layout_arena) = json.pointer("/game/arena").or_else(|| json.get("arena")).and_then(|v| v.as_str()) {
                    if layout_arena != arena_id {
                        continue;
                    }
                }

                let raw_terrain = json.pointer("/game/game/terrain")
                    .or_else(|| json.pointer("/game/terrain"))
                    .or_else(|| json.get("terrain"))
                    .and_then(|v| v.as_str());

                if let Some(terrain_str) = raw_terrain {
                    valid_grids.push((path, crate::models::Terrain::parse_string(terrain_str, width_u, height_u)));
                }
            }
        }
    }

    if valid_grids.is_empty() {
        return Err(anyhow::anyhow!("No available layouts found for arena ID '{}'. Game cannot be launched.", arena_id));
    }

    // Pick a layout at random
    let idx = fastrand::usize(..valid_grids.len());
    let (chosen_path, grid) = valid_grids.remove(idx);
    println!("Loaded terrain layout from: {:?}", chosen_path);

    Ok(grid)
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ArenaAlias {
    pub arena_id: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum AliasValue {
    Simple(String),
    Full(ArenaAlias),
}

impl AliasValue {
    pub fn into_arena_alias(self) -> ArenaAlias {
        match self {
            AliasValue::Simple(arena_id) => ArenaAlias {
                arena_id,
                name: String::new(),
            },
            AliasValue::Full(alias) => alias,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BotEntry {
    pub id: u32,
    pub name: String,
    pub version: u32,
    pub arena_id: String,
    pub path: String,
    pub sha256: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BotLibrary {
    pub next_id: u32,
    pub bots: Vec<BotEntry>,
    #[serde(default, deserialize_with = "deserialize_aliases")]
    pub aliases: HashMap<String, ArenaAlias>, // alias -> ArenaAlias
}

fn deserialize_aliases<'de, D>(deserializer: D) -> Result<HashMap<String, ArenaAlias>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: HashMap<String, AliasValue> = serde::Deserialize::deserialize(deserializer)?;
    Ok(raw.into_iter().map(|(k, v)| (k, v.into_arena_alias())).collect())
}

impl Default for BotLibrary {
    fn default() -> Self {
        Self {
            next_id: 1,
            bots: Vec::new(),
            aliases: HashMap::new(),
        }
    }
}

fn calculate_sha256(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).context("Failed to open file for hashing")?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).context("Failed to read file for hashing")?;
    let result = hasher.finalize();
    Ok(format!("{:x}", result))
}

impl BotLibrary {
    pub fn load(dir: &Path) -> Result<Self> {
        let meta_path = dir.join("bot_library.json");
        if !meta_path.exists() {
            return Ok(BotLibrary::default());
        }
        let content = fs::read_to_string(&meta_path)
            .context("Failed to read bot library metadata file")?;
        let mut lib: BotLibrary = serde_json::from_str(&content)
            .context("Failed to parse bot library metadata JSON")?;
        if lib.next_id == 0 {
            lib.next_id = 1;
        }
        Ok(lib)
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        fs::create_dir_all(dir).context("Failed to create bot library directory")?;
        let meta_path = dir.join("bot_library.json");
        let content = serde_json::to_string_pretty(self)
            .context("Failed to serialize bot library metadata")?;
        fs::write(meta_path, content)
            .context("Failed to write bot library metadata file")?;
        Ok(())
    }

    /// Resolves an arena input string (which can be an arena_id or an alias) to the canonical arena_id.
    pub fn resolve_arena_id(&self, input: &str) -> String {
        let trimmed = input.trim();
        if let Some(target) = self.aliases.get(trimmed) {
            target.arena_id.clone()
        } else {
            trimmed.to_string()
        }
    }

    pub fn set_alias(&mut self, dir: &Path, alias: &str, arena_id: &str, name: Option<&str>) -> Result<()> {
        let alias = alias.trim().to_string();
        let arena_id = arena_id.trim().to_string();

        if alias.is_empty() {
            return Err(anyhow::anyhow!("Alias cannot be empty"));
        }
        if arena_id.is_empty() {
            return Err(anyhow::anyhow!("Arena ID cannot be empty"));
        }

        let name = name.unwrap_or("").trim().to_string();

        self.aliases.insert(alias, ArenaAlias { arena_id, name });
        self.save(dir)?;
        Ok(())
    }

    pub fn remove_alias(&mut self, dir: &Path, alias: &str) -> Result<()> {
        let alias = alias.trim();
        if self.aliases.remove(alias).is_none() {
            return Err(anyhow::anyhow!("Alias '{}' not found", alias));
        }
        self.save(dir)?;
        Ok(())
    }

    pub fn add(&mut self, dir: &Path, name: &str, arena_or_alias: &str, source_path: &Path) -> Result<BotEntry> {
        let arena_id = self.resolve_arena_id(arena_or_alias);
        if arena_id.is_empty() {
            return Err(anyhow::anyhow!("Arena ID / alias cannot be empty"));
        }

        let sha256 = calculate_sha256(source_path)?;

        // Check if a bot with the same SHA256 already exists
        if let Some(existing) = self.bots.iter().find(|b| b.sha256 == sha256) {
            return Err(anyhow::anyhow!(
                "A bot binary with the same SHA256 hash already exists in the library as '{}:{}' (ID: {})",
                existing.name, existing.version, existing.id
            ));
        }

        // Find next version for this bot name
        let version = self.bots.iter()
            .filter(|b| b.name == name)
            .map(|b| b.version)
            .max()
            .map(|v| v + 1)
            .unwrap_or(0);

        let id = self.next_id;
        self.next_id += 1;

        let extension = source_path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("so");
        let dest_filename = format!("bot_{}.{}", id, extension);
        let dest_path = dir.join(&dest_filename);

        fs::create_dir_all(dir).context("Failed to create bot library directory")?;
        fs::copy(source_path, &dest_path)
            .with_context(|| format!("Failed to copy bot binary from {:?} to {:?}", source_path, dest_path))?;

        let entry = BotEntry {
            id,
            name: name.to_string(),
            version,
            arena_id,
            path: dest_path.to_string_lossy().to_string(),
            sha256,
        };

        self.bots.push(entry.clone());
        self.save(dir)?;
        Ok(entry)
    }

    pub fn rename(&mut self, dir: &Path, old_name: &str, new_name: &str) -> Result<()> {
        let mut matches = 0;
        for bot in &mut self.bots {
            if bot.name == old_name {
                bot.name = new_name.to_string();
                matches += 1;
            }
        }
        if matches == 0 {
            return Err(anyhow::anyhow!("No bots found with name '{}'", old_name));
        }
        self.save(dir)?;
        Ok(())
    }

    pub fn delete(&mut self, _dir: &Path, query: &str) -> Result<()> {
        // Check if query is name:version or just name
        let to_remove: Vec<usize> = if query.contains(':') {
            let parts: Vec<&str> = query.split(':').collect();
            let name = parts[0];
            let version: u32 = parts[1].parse().context("Invalid version number in delete query")?;
            self.bots.iter().enumerate()
                .filter(|(_, b)| b.name == name && b.version == version)
                .map(|(idx, _)| idx)
                .collect()
        } else {
            self.bots.iter().enumerate()
                .filter(|(_, b)| b.name == query)
                .map(|(idx, _)| idx)
                .collect()
        };

        if to_remove.is_empty() {
            return Err(anyhow::anyhow!("No bots matching '{}' found in library", query));
        }

        // Remove files and entries in reverse order
        for idx in to_remove.into_iter().rev() {
            let entry = self.bots.remove(idx);
            let path = PathBuf::from(entry.path);
            if path.exists() {
                let _ = fs::remove_file(&path);
            }
        }

        self.save(_dir)?;
        Ok(())
    }
}

