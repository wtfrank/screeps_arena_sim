//! # Screeps Arena Simulator CLI
//!
//! This binary provides administrative operations for managing the local
//! bot library and executing arena simulations.

use std::path::PathBuf;
use clap::{Parser, Subcommand};
use anyhow::{Context, Result};

mod models;
mod bot_library;
mod driver;
mod executor;

use models::{GameState, Position, GameObject, Owner, Terrain, Ruleset, WinCondition};

#[derive(Parser)]
#[command(name = "screeps_arena_sim")]
#[command(about = "Screeps Arena Local Simulation & Test Harness")]
struct Cli {
    /// Directory path to store the bot library [default: $XDG_DATA_HOME/screeps_arena_sim or ~/.local/share/screeps_arena_sim]
    #[arg(short, long)]
    library_dir: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage real arenas
    Arena {
        #[command(subcommand)]
        action: ArenaCommands,
    },
    /// Manage arena aliases
    Alias {
        #[command(subcommand)]
        action: AliasCommands,
    },
    /// Manage the bot library
    Lib {
        #[command(subcommand)]
        action: LibCommands,
    },
    /// Run a simulation match between two bots
    Run {
        /// Name:version or ID of the first bot (e.g. wtfbot:0)
        bot1: String,
        /// Name:version or ID of the second bot (e.g. wtfbot:1)
        bot2: String,
        /// Real arena ID or short alias
        arena: String,
        /// Maximum ticks to simulate
        #[arg(short, long, default_value_t = 1000)]
        ticks: u32,
    },
}

#[derive(Subcommand)]
enum ArenaCommands {
    /// List known arenas
    List,
}

#[derive(Subcommand)]
enum AliasCommands {
    /// Set a short alias for a real arena ID
    Set {
        /// Short unique alias (e.g. ssb)
        alias: String,
        /// Real arena ID (e.g. 69cfe6fcece2ae9f75da12d1)
        arena_id: String,
        /// Optional human-readable name of the arena (e.g. "Spawn Strike 3")
        #[arg(short, long)]
        name: Option<String>,
    },
    /// Remove an arena alias
    Remove {
        /// Short alias to remove
        alias: String,
    },
    /// List all defined arena aliases
    List,
}

#[derive(Subcommand)]
enum LibCommands {
    /// Add a compiled bot binary to the library
    Add {
        /// Visible name of the bot family
        name: String,
        /// Real arena ID or short alias
        arena: String,
        /// Path to the compiled dynamic library (.so or .dll)
        path: PathBuf,
    },
    /// Rename a bot family in the library
    Rename {
        /// Current name of the bot family
        old_name: String,
        /// New name for the bot family
        new_name: String,
    },
    /// Delete a bot family or a specific version (e.g. name:version) from the library
    Delete {
        /// Visible name (e.g. wtfbot) or fully specified version (e.g. wtfbot:2)
        name_or_version: String,
    },
    /// List all bots and revisions in the library
    List,
}

fn get_default_library_dir() -> PathBuf {
    if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME") {
        if !xdg_data.is_empty() {
            return PathBuf::from(xdg_data).join("screeps_arena_sim");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home).join(".local").join("share").join("screeps_arena_sim");
        }
    }
    PathBuf::from("./bot_library")
}

fn get_cache_dir() -> PathBuf {
    if let Ok(xdg_cache) = std::env::var("XDG_CACHE_HOME") {
        if !xdg_cache.is_empty() {
            return PathBuf::from(xdg_cache).join("screeps_arena_sim");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home).join(".cache").join("screeps_arena_sim");
        }
    }
    PathBuf::from("./bot_cache")
}

fn generate_default_state(library_dir: &std::path::Path, arena_id: &str, width: u8, height: u8) -> Result<GameState> {
    let mut objects = Vec::new();

    // Spawn 1 for Bot1, Spawn 2 for Bot2
    objects.push(GameObject::Spawn {
        id: "spawn1".to_string(),
        pos: Position { x: 10, y: height / 2 },
        hits: 5000,
        max_hits: 5000,
        owner: Owner::Bot1,
        energy: 1000,
        max_energy: 1000,
    });
    objects.push(GameObject::Spawn {
        id: "spawn2".to_string(),
        pos: Position { x: width - 11, y: height / 2 },
        hits: 5000,
        max_hits: 5000,
        owner: Owner::Bot2,
        energy: 1000,
        max_energy: 1000,
    });

    // A few initial creeps for testing
    objects.push(GameObject::Creep {
        id: "creep1".to_string(),
        pos: Position { x: 12, y: height / 2 },
        hits: 100,
        max_hits: 100,
        owner: Owner::Bot1,
        fatigue: 0,
    });
    objects.push(GameObject::Creep {
        id: "creep2".to_string(),
        pos: Position { x: width - 13, y: height / 2 },
        hits: 100,
        max_hits: 100,
        owner: Owner::Bot2,
        fatigue: 0,
    });

    let terrain = bot_library::load_arena_terrain(library_dir, arena_id, width, height)?;

    Ok(GameState {
        tick: 1,
        width,
        height,
        objects,
        terrain,
    })
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    
    let library_path = cli.library_dir
        .map(PathBuf::from)
        .unwrap_or_else(get_default_library_dir);

    match cli.command {
        Commands::Arena { action } => match action {
            ArenaCommands::List => {
                let arenas = bot_library::get_known_arenas();
                println!("{:<30} | {:<25} | {:<10} | {}", "Arena Name", "Folder Name", "Advanced", "Arena ID");
                println!("{}", "-".repeat(95));
                for arena in arenas {
                    println!("{:<30} | {:<25} | {:<10} | {}", arena.name, arena.folder_name, arena.advanced, arena.arena_id);
                }
            }
        },
        Commands::Alias { action } => match action {
            AliasCommands::Set { alias, arena_id, name } => {
                let mut lib = bot_library::BotLibrary::load(&library_path)?;
                lib.set_alias(&library_path, &alias, &arena_id, name.as_deref())?;
                if let Some(ref n) = name {
                    println!("Successfully set alias '{}' -> '{}' ({})", alias, arena_id, n);
                } else {
                    println!("Successfully set alias '{}' -> '{}'", alias, arena_id);
                }
            }
            AliasCommands::Remove { alias } => {
                let mut lib = bot_library::BotLibrary::load(&library_path)?;
                lib.remove_alias(&library_path, &alias)?;
                println!("Successfully removed alias '{}'", alias);
            }
            AliasCommands::List => {
                let lib = bot_library::BotLibrary::load(&library_path)?;
                if lib.aliases.is_empty() {
                    println!("No arena aliases defined.");
                } else {
                    println!("{:<15} | {:<25} | {}", "Alias", "Arena Name", "Arena ID");
                    println!("{}", "-".repeat(70));
                    for (alias, target) in &lib.aliases {
                        let name_display = if target.name.is_empty() { "-" } else { &target.name };
                        println!("{:<15} | {:<25} | {}", alias, name_display, target.arena_id);
                    }
                }
            }
        },
        Commands::Lib { action } => match action {
            LibCommands::Add { name, arena, path } => {
                let mut lib = bot_library::BotLibrary::load(&library_path)?;
                let entry = lib.add(&library_path, &name, &arena, &path)?;
                println!("Successfully added bot '{}:{}' (ID: {}) linked to arena ID '{}'", entry.name, entry.version, entry.id, entry.arena_id);
            }
            LibCommands::Rename { old_name, new_name } => {
                let mut lib = bot_library::BotLibrary::load(&library_path)?;
                lib.rename(&library_path, &old_name, &new_name)?;
                println!("Successfully renamed all bot revisions named '{}' to '{}'", old_name, new_name);
            }
            LibCommands::Delete { name_or_version } => {
                let mut lib = bot_library::BotLibrary::load(&library_path)?;
                lib.delete(&library_path, &name_or_version)?;
                println!("Successfully deleted '{}' from the library", name_or_version);
            }
            LibCommands::List => {
                let lib = bot_library::BotLibrary::load(&library_path)?;
                if lib.bots.is_empty() {
                    println!("The bot library is empty.");
                } else {
                    println!("{:<5} | {:<20} | {:<30} | {}", "ID", "Visible Name", "Arena Link", "Binary Path");
                    println!("{}", "-".repeat(90));
                    for bot in lib.bots {
                        let visible_name = format!("{}:{}", bot.name, bot.version);
                        println!("{:<5} | {:<20} | {:<30} | {}", bot.id, visible_name, bot.arena_id, bot.path);
                    }
                }
            }
        },
        Commands::Run { bot1, bot2, arena, ticks } => {
            let lib = bot_library::BotLibrary::load(&library_path)?;
            let arena_id = lib.resolve_arena_id(&arena);
            
            // Resolve Bot 1 path
            let path1 = if let Ok(id) = bot1.parse::<u32>() {
                lib.bots.iter().find(|b| b.id == id).map(|b| &b.path)
            } else {
                let parts: Vec<&str> = bot1.split(':').collect();
                if parts.len() == 2 {
                    let version = parts[1].parse::<u32>().unwrap_or(0);
                    lib.bots.iter().find(|b| b.name == parts[0] && b.version == version).map(|b| &b.path)
                } else {
                    lib.bots.iter().filter(|b| b.name == bot1).max_by_key(|b| b.version).map(|b| &b.path)
                }
            };
            let bot1_path = path1.context(format!("Failed to find Bot 1 matching: {}", bot1))?;

            // Resolve Bot 2 path
            let path2 = if let Ok(id) = bot2.parse::<u32>() {
                lib.bots.iter().find(|b| b.id == id).map(|b| &b.path)
            } else {
                let parts: Vec<&str> = bot2.split(':').collect();
                if parts.len() == 2 {
                    let version = parts[1].parse::<u32>().unwrap_or(0);
                    lib.bots.iter().find(|b| b.name == parts[0] && b.version == version).map(|b| &b.path)
                } else {
                    lib.bots.iter().filter(|b| b.name == bot2).max_by_key(|b| b.version).map(|b| &b.path)
                }
            };
            let bot2_path = path2.context(format!("Failed to find Bot 2 matching: {}", bot2))?;

            println!("Loading Bot 1: {:?}", bot1_path);
            println!("Loading Bot 2: {:?}", bot2_path);

            let initial_state = generate_default_state(&library_path, &arena_id, 100, 100)?;
            let rules = Ruleset {
                tick_limit: ticks,
                cpu_time_limit: 1000,
                win_condition: WinCondition::DestroyEnemySpawn,
            };

            // dlopen() (used by libloading) caches library handles by file path: loading the same
            // .so path twice in one process returns the same handle, sharing all global static memory
            // between both bots. We copy each bot to a unique path in the user-owned XDG cache
            // directory so they get independent address spaces. We use the process ID to avoid
            // collisions with concurrent simulator instances.
            let cache_dir = get_cache_dir();
            std::fs::create_dir_all(&cache_dir).context("Failed to create cache directory")?;
            let pid = std::process::id();
            let p1 = cache_dir.join(format!("bot_1_{}.so", pid));
            let p2 = cache_dir.join(format!("bot_2_{}.so", pid));
            std::fs::copy(std::path::Path::new(bot1_path), &p1)
                .context("Failed to copy Bot 1 to cache")?;
            std::fs::copy(std::path::Path::new(bot2_path), &p2)
                .context("Failed to copy Bot 2 to cache")?;

            let mut executor = executor::RunExecutor::new(initial_state, &p1, &p2, rules)?;

            println!("Starting simulation on arena ID '{}'...", arena_id);
            loop {
                match executor.step_tick()? {
                    Some(result) => {
                        println!("Simulation finished! Result: {:?}", result);
                        break;
                    }
                    None => {
                        // Keep going
                    }
                }
            }
        }
    }

    Ok(())
}

