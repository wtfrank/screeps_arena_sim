use std::path::{Path, PathBuf};
use std::fs;
use anyhow::{Context, Result};
use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BotEntry {
    pub id: u32,
    pub name: String,
    pub version: u32,
    pub map: String,
    pub path: String,
    pub sha256: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BotLibrary {
    pub next_id: u32,
    pub bots: Vec<BotEntry>,
}

impl Default for BotLibrary {
    fn default() -> Self {
        Self {
            next_id: 1,
            bots: Vec::new(),
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

/// Standardizes map input and validates it against the 6 allowed maps.
fn validate_and_standardize_map(map_input: &str) -> Result<String> {
    match map_input.to_lowercase().as_str() {
        "spawn_strike_basic" | "ssb" => Ok("spawn_strike_basic".to_string()),
        "spawn_strike_advanced" | "ssa" => Ok("spawn_strike_advanced".to_string()),
        "power_split_basic" | "psb" => Ok("power_split_basic".to_string()),
        "power_split_advanced" | "psa" => Ok("power_split_advanced".to_string()),
        "escort_run_basic" | "erb" => Ok("escort_run_basic".to_string()),
        "escort_run_advanced" | "era" => Ok("escort_run_advanced".to_string()),
        _ => Err(anyhow::anyhow!(
            "Invalid map '{}'. Allowed maps are:\n\
             - spawn_strike_basic (alias: ssb)\n\
             - spawn_strike_advanced (alias: ssa)\n\
             - power_split_basic (alias: psb)\n\
             - power_split_advanced (alias: psa)\n\
             - escort_run_basic (alias: erb)\n\
             - escort_run_advanced (alias: era)",
            map_input
        )),
    }
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

    pub fn add(&mut self, dir: &Path, name: &str, map: &str, source_path: &Path) -> Result<BotEntry> {
        let standardized_map = validate_and_standardize_map(map)?;
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
            map: standardized_map,
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
