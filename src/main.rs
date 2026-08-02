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
    /// Manage layout aliases
    Layout {
        #[command(subcommand)]
        action: LayoutAliasCommands,
    },
    /// Manage the bot library
    Bot {
        #[command(subcommand)]
        action: BotCommands,
    },
    /// Run a simulation match with 1 or 2 bots
    Run {
        /// Real arena ID or short alias
        arena: String,
        /// Name:version or ID of the first bot (e.g. wtfbot:0)
        bot1: String,
        /// Optional name:version or ID of the second bot (e.g. wtfbot:1)
        bot2: Option<String>,
        /// Optional layout ID or layout alias to use instead of random selection
        #[arg(short, long)]
        layout: Option<String>,
        /// Maximum ticks to simulate
        #[arg(short, long, default_value_t = 1000)]
        ticks: u32,
        /// Launch GDB terminal wrapper for debugging a specific bot (1, 2, or "all")
        #[arg(long)]
        debug_bot: Option<String>,
    },
    /// Internal worker process command for executing a bot in an isolated process over IPC
    #[command(hide = true)]
    BotRunner {
        /// Path to the bot binary .so file
        bot_path: String,
        /// Unix socket FD for IPC communication
        socket_fd: i32,
        /// Pause process before calling bot_initialize() for debugger attachment
        #[arg(long)]
        pause: bool,
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
enum LayoutAliasCommands {
    /// Set a short alias for a layout file or target
    Set {
        /// Short unique layout alias (e.g. map1)
        alias: String,
        /// Target layout filename or path
        layout: String,
    },
    /// Remove a layout alias
    Remove {
        /// Short layout alias to remove
        alias: String,
    },
    /// List all defined layout aliases
    List,
}

#[derive(Subcommand)]
enum BotCommands {
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

fn load_state(
    library_dir: &std::path::Path,
    arena_id: &str,
    specified_layout: Option<&str>,
    layout_aliases: &std::collections::HashMap<String, String>,
    arena_aliases: &std::collections::HashMap<String, bot_library::ArenaAlias>,
    width: u8,
    height: u8,
) -> Result<GameState> {

    let (terrain, objects) = bot_library::load_arena_terrain(
        library_dir,
        arena_id,
        specified_layout,
        layout_aliases,
        arena_aliases,
        width,
        height,
    )?;

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
        Commands::Layout { action } => match action {
            LayoutAliasCommands::Set { alias, layout } => {
                let mut lib = bot_library::BotLibrary::load(&library_path)?;
                lib.set_layout_alias(&library_path, &alias, &layout)?;
                println!("Successfully set layout alias '{}' -> '{}'", alias, layout);
            }
            LayoutAliasCommands::Remove { alias } => {
                let mut lib = bot_library::BotLibrary::load(&library_path)?;
                lib.remove_layout_alias(&library_path, &alias)?;
                println!("Successfully removed layout alias '{}'", alias);
            }
            LayoutAliasCommands::List => {
                let lib = bot_library::BotLibrary::load(&library_path)?;
                let layouts = bot_library::list_all_layouts(&library_path, &lib.layout_aliases);
                if layouts.is_empty() {
                    println!("No layout files found in layouts directory.");
                } else {
                    println!("{:<15} | {:<28} | {:<26} | {}", "Layout Alias", "Game ID", "Arena ID", "Arena Name");
                    println!("{}", "-".repeat(100));
                    for layout in layouts {
                        let alias_display = layout.alias.as_deref().unwrap_or("-");
                        println!("{:<15} | {:<28} | {:<26} | {}", alias_display, layout.game_id, layout.arena_id, layout.arena_name);
                    }
                }
            }
        },
        Commands::Bot { action } => match action {
            BotCommands::Add { name, arena, path } => {
                let mut lib = bot_library::BotLibrary::load(&library_path)?;
                let entry = lib.add(&library_path, &name, &arena, &path)?;
                println!("Successfully added bot '{}:{}' (ID: {}) linked to arena ID '{}'", entry.name, entry.version, entry.id, entry.arena_id);
            }
            BotCommands::Rename { old_name, new_name } => {
                let mut lib = bot_library::BotLibrary::load(&library_path)?;
                lib.rename(&library_path, &old_name, &new_name)?;
                println!("Successfully renamed all bot revisions named '{}' to '{}'", old_name, new_name);
            }
            BotCommands::Delete { name_or_version } => {
                let mut lib = bot_library::BotLibrary::load(&library_path)?;
                lib.delete(&library_path, &name_or_version)?;
                println!("Successfully deleted '{}' from the library", name_or_version);
            }
            BotCommands::List => {
                let lib = bot_library::BotLibrary::load(&library_path)?;
                if lib.bots.is_empty() {
                    println!("The bot library is empty.");
                } else {
                    println!("{:<5} | {:<20} | {:<25} | {:<8} | {:<8} | {}", "ID", "Visible Name", "Arena Link", "Stable", "Crashes", "Binary Path");
                    println!("{}", "-".repeat(110));
                    for bot in lib.bots {
                        let visible_name = format!("{}:{}", bot.name, bot.version);
                        println!("{:<5} | {:<20} | {:<25} | {:<8} | {:<8} | {}", bot.id, visible_name, bot.arena_id, bot.stable_count, bot.crash_count, bot.path);
                    }
                }
            }
        },
        Commands::Run { arena, bot1, bot2, layout, ticks, debug_bot } => {
            let lib = bot_library::BotLibrary::load(&library_path)?;
            let arena_id = lib.resolve_arena_id(&arena);
            
            // Resolve Bot 1
            let bot1_entry = if let Ok(id) = bot1.parse::<u32>() {
                lib.bots.iter().find(|b| b.id == id)
            } else {
                let parts: Vec<&str> = bot1.split(':').collect();
                if parts.len() == 2 {
                    let version = parts[1].parse::<u32>().unwrap_or(0);
                    lib.bots.iter().find(|b| b.name == parts[0] && b.version == version)
                } else {
                    lib.bots.iter().filter(|b| b.name == bot1).max_by_key(|b| b.version)
                }
            }.context(format!("Failed to find Bot 1 matching: {}", bot1))?;
            let bot1_id = bot1_entry.id;
            let bot1_path = &bot1_entry.path;

            // Resolve optional Bot 2
            let bot2_resolved = if let Some(ref b2_str) = bot2 {
                let entry = if let Ok(id) = b2_str.parse::<u32>() {
                    lib.bots.iter().find(|b| b.id == id)
                } else {
                    let parts: Vec<&str> = b2_str.split(':').collect();
                    if parts.len() == 2 {
                        let version = parts[1].parse::<u32>().unwrap_or(0);
                        lib.bots.iter().find(|b| b.name == parts[0] && b.version == version)
                    } else {
                        lib.bots.iter().filter(|b| b.name == b2_str.as_str()).max_by_key(|b| b.version)
                    }
                }.context(format!("Failed to find Bot 2 matching: {}", b2_str))?;
                Some((entry.id, entry.path.clone()))
            } else {
                None
            };

            println!("Loading Bot 1: {:?}", bot1_path);
            if let Some((_, ref p2)) = bot2_resolved {
                println!("Loading Bot 2: {:?}", p2);
            } else {
                println!("No Bot 2 specified: running single-bot simulation");
            }

            let initial_state = load_state(&library_path, &arena_id, layout.as_deref(), &lib.layout_aliases, &lib.aliases, 100, 100)?;
            let rules = Ruleset {
                tick_limit: ticks,
                cpu_time_limit: 1000,
                win_condition: WinCondition::DestroyEnemySpawn,
            };

            let p2_opt = bot2_resolved.as_ref().map(|(_, path)| std::path::Path::new(path));
            let mut executor = executor::RunExecutor::new(
                initial_state,
                std::path::Path::new(bot1_path),
                p2_opt,
                rules,
                debug_bot.as_deref(),
            )?;

            println!("Starting simulation on arena ID '{}'...", arena_id);
            loop {
                match executor.step_tick()? {
                    Some(result) => {
                        println!("Simulation finished! Result: {:?}", result);

                        // Record crash and stable metrics in BotLibrary
                        let mut update_lib = bot_library::BotLibrary::load(&library_path)?;
                        if executor.bot1_crashed() {
                            update_lib.record_crash(&library_path, bot1_id)?;
                        } else {
                            update_lib.record_stable(&library_path, bot1_id)?;
                        }

                        if let Some((b2_id, _)) = bot2_resolved {
                            if executor.bot2_crashed() {
                                update_lib.record_crash(&library_path, b2_id)?;
                            } else {
                                update_lib.record_stable(&library_path, b2_id)?;
                            }
                        }

                        break;
                    }
                    None => {
                        // Keep going
                    }
                }
            }
        }
        Commands::BotRunner { bot_path, socket_fd, pause } => {
            driver::run_bot_runner_process(&bot_path, socket_fd, pause)?;
        }
    }

    Ok(())
}

