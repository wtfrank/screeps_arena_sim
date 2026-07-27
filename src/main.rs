//! # Screeps Arena Simulator CLI
//!
//! This binary provides administrative operations for managing the local
//! bot library and executing arena simulations.

use std::path::PathBuf;
use clap::{Parser, Subcommand};
use anyhow::Result;

mod models;
mod bot_library;

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
    /// Manage the bot library
    Lib {
        #[command(subcommand)]
        action: LibCommands,
    },
}

#[derive(Subcommand)]
enum LibCommands {
    /// Add a compiled bot binary to the library
    Add {
        /// Visible name of the bot family
        name: String,
        /// Map association (e.g. spawn_strike_basic)
        map: String,
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

fn main() -> Result<()> {
    let cli = Cli::parse();
    
    let library_path = cli.library_dir
        .map(PathBuf::from)
        .unwrap_or_else(get_default_library_dir);

    match cli.command {
        Commands::Lib { action } => match action {
            LibCommands::Add { name, map, path } => {
                let mut lib = bot_library::BotLibrary::load(&library_path)?;
                let entry = lib.add(&library_path, &name, &map, &path)?;
                println!("Successfully added bot '{}:{}' (ID: {}) linked to map '{}'", entry.name, entry.version, entry.id, map);
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
                    println!("{:<5} | {:<20} | {:<20} | {}", "ID", "Visible Name", "Map Association", "Binary Path");
                    println!("{}", "-".repeat(80));
                    for bot in lib.bots {
                        let visible_name = format!("{}:{}", bot.name, bot.version);
                        println!("{:<5} | {:<20} | {:<20} | {}", bot.id, visible_name, bot.map, bot.path);
                    }
                }
            }
        },
    }

    Ok(())
}
