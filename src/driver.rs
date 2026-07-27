use std::os::raw::{c_char, c_void};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use anyhow::{Context, Result};
use libloading::{Library, Symbol};

use screeps_arena::ffi::{HostInterface, ActionId, PrototypeId};

struct SendPtr(pub *mut c_void);
unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

// Thread-local context to identify which bot is executing and access its state queries.
thread_local! {
    static CURRENT_BOT_CONTEXT: std::cell::RefCell<Option<BotExecutionContext>> = std::cell::RefCell::new(None);
}

/// Active state context exposed to FFI callbacks on a per-thread basis.
struct BotExecutionContext {
    pub tick: u32,
    pub is_bot_1: bool,
    pub action_queue: Arc<Mutex<Vec<QueuedAction>>>,
    // Thread-local caches to ensure raw pointers returned to FFI remain valid during the tick.
    pub creep_cache: Vec<screeps_arena::objects::Creep>,
    pub spawn_cache: Vec<screeps_arena::objects::StructureSpawn>,
    pub tower_cache: Vec<screeps_arena::objects::StructureTower>,
    pub extension_cache: Vec<screeps_arena::objects::StructureExtension>,
    pub rampart_cache: Vec<screeps_arena::objects::StructureRampart>,
    pub container_cache: Vec<screeps_arena::objects::StructureContainer>,
    pub road_cache: Vec<screeps_arena::objects::StructureRoad>,
    pub wall_cache: Vec<screeps_arena::objects::StructureWall>,
    pub resource_cache: Vec<screeps_arena::objects::Resource>,
    pub source_cache: Vec<screeps_arena::objects::Source>,
    pub flag_cache: Vec<screeps_arena::objects::Flag>,
    pub score_collector_cache: Vec<screeps_arena::objects::ScoreCollector>,
    pub bonus_flag_cache: Vec<screeps_arena::objects::BonusFlag>,
    pub area_effect_cache: Vec<screeps_arena::objects::AreaEffect>,
    pub construction_site_cache: Vec<screeps_arena::objects::ConstructionSite>,
}

#[derive(Debug, Clone)]
pub struct QueuedAction {
    pub actor_id: String,
    pub action: ActionId,
    pub target_id: Option<String>,
    pub arg1: usize,
    pub arg2: usize,
}

/// Direct FFI callbacks routed from the bot back to the simulator host
extern "C" fn get_ticks_callback() -> u32 {
    CURRENT_BOT_CONTEXT.with(|ctx| {
        ctx.borrow().as_ref().map(|c| c.tick).unwrap_or(0)
    })
}

extern "C" fn get_cpu_time_callback() -> u32 {
    // Return dummy CPU elapsed time (or track actual thread CPU execution time)
    0
}

extern "C" fn get_terrain_at_callback(_x: u8, _y: u8) -> u32 {
    // Default to Plain (0) for now. The full executor will wire this up to the actual terrain grid.
    0
}

extern "C" fn get_objects_callback(proto: u32, out_ptr: *mut *const c_void, out_len: *mut usize) {
    CURRENT_BOT_CONTEXT.with(|ctx| {
        if let Some(ref c) = *ctx.borrow() {
            unsafe {
                match proto {
                    1 => { // Creep
                        *out_ptr = c.creep_cache.as_ptr() as *const c_void;
                        *out_len = c.creep_cache.len();
                    }
                    2 => { // StructureSpawn
                        *out_ptr = c.spawn_cache.as_ptr() as *const c_void;
                        *out_len = c.spawn_cache.len();
                    }
                    3 => { // StructureTower
                        *out_ptr = c.tower_cache.as_ptr() as *const c_void;
                        *out_len = c.tower_cache.len();
                    }
                    4 => { // StructureExtension
                        *out_ptr = c.extension_cache.as_ptr() as *const c_void;
                        *out_len = c.extension_cache.len();
                    }
                    5 => { // StructureRampart
                        *out_ptr = c.rampart_cache.as_ptr() as *const c_void;
                        *out_len = c.rampart_cache.len();
                    }
                    6 => { // StructureContainer
                        *out_ptr = c.container_cache.as_ptr() as *const c_void;
                        *out_len = c.container_cache.len();
                    }
                    7 => { // StructureRoad
                        *out_ptr = c.road_cache.as_ptr() as *const c_void;
                        *out_len = c.road_cache.len();
                    }
                    8 => { // StructureWall
                        *out_ptr = c.wall_cache.as_ptr() as *const c_void;
                        *out_len = c.wall_cache.len();
                    }
                    9 => { // Resource
                        *out_ptr = c.resource_cache.as_ptr() as *const c_void;
                        *out_len = c.resource_cache.len();
                    }
                    10 => { // Source
                        *out_ptr = c.source_cache.as_ptr() as *const c_void;
                        *out_len = c.source_cache.len();
                    }
                    11 => { // Flag
                        *out_ptr = c.flag_cache.as_ptr() as *const c_void;
                        *out_len = c.flag_cache.len();
                    }
                    12 => { // ScoreCollector
                        *out_ptr = c.score_collector_cache.as_ptr() as *const c_void;
                        *out_len = c.score_collector_cache.len();
                    }
                    13 => { // BonusFlag
                        *out_ptr = c.bonus_flag_cache.as_ptr() as *const c_void;
                        *out_len = c.bonus_flag_cache.len();
                    }
                    14 => { // AreaEffect
                        *out_ptr = c.area_effect_cache.as_ptr() as *const c_void;
                        *out_len = c.area_effect_cache.len();
                    }
                    15 => { // ConstructionSite
                        *out_ptr = c.construction_site_cache.as_ptr() as *const c_void;
                        *out_len = c.construction_site_cache.len();
                    }
                    _ => {
                        *out_ptr = std::ptr::null();
                        *out_len = 0;
                    }
                }
            }
        }
    });
}

extern "C" fn queue_action_callback(
    actor_id: *const c_char,
    action: u32,
    target_id: *const c_char,
    arg1: usize,
    arg2: usize,
) {
    unsafe {
        let actor = std::ffi::CStr::from_ptr(actor_id).to_string_lossy().into_owned();
        let target = if target_id.is_null() {
            None
        } else {
            Some(std::ffi::CStr::from_ptr(target_id).to_string_lossy().into_owned())
        };

        if let Some(action_enum) = ActionId::from_u32(action) {
            CURRENT_BOT_CONTEXT.with(|ctx| {
                if let Some(ref c) = *ctx.borrow() {
                    let mut queue = c.action_queue.lock().unwrap();
                    queue.push(QueuedAction {
                        actor_id: actor,
                        action: action_enum,
                        target_id: target,
                        arg1,
                        arg2,
                    });
                }
            });
        }
    }
}

pub struct BotDriver {
    _lib: Library,
    bot_ptr: *mut c_void,
    bot_tick_fn: Symbol<'static, unsafe extern "C" fn(*mut c_void)>,
    bot_free_fn: Symbol<'static, unsafe extern "C" fn(*mut c_void)>,
}

unsafe impl Send for BotDriver {}
unsafe impl Sync for BotDriver {}

impl BotDriver {
    pub fn load(path: &Path) -> Result<Self> {
        let lib = unsafe { Library::new(path).context("Failed to dynamically load bot library binary")? };

        // Bind lifecycle symbols
        let set_host_interface_fn: Symbol<unsafe extern "C" fn(HostInterface)> = unsafe {
            lib.get(b"set_host_interface").context("Failed to bind set_host_interface symbol")?
        };
        let bot_initialize_fn: Symbol<unsafe extern "C" fn() -> *mut c_void> = unsafe {
            lib.get(b"bot_initialize").context("Failed to bind bot_initialize symbol")?
        };
        let bot_tick_fn: Symbol<unsafe extern "C" fn(*mut c_void)> = unsafe {
            lib.get(b"bot_tick").context("Failed to bind bot_tick symbol")?
        };
        let bot_free_fn: Symbol<unsafe extern "C" fn(*mut c_void)> = unsafe {
            lib.get(b"bot_free").context("Failed to bind bot_free symbol")?
        };

        // Initialize host callbacks inside the bot DLL
        let interface = HostInterface {
            get_ticks: get_ticks_callback,
            get_cpu_time: get_cpu_time_callback,
            get_objects: get_objects_callback,
            get_terrain_at: get_terrain_at_callback,
            queue_action: queue_action_callback,
        };

        unsafe {
            set_host_interface_fn(interface);
        }

        // Initialize the bot state (Tick 0)
        let bot_ptr = unsafe { bot_initialize_fn() };

        // Transmute symbols to extend their lifetime to match the struct ownership
        let bot_tick_fn = unsafe { std::mem::transmute(bot_tick_fn) };
        let bot_free_fn = unsafe { std::mem::transmute(bot_free_fn) };

        Ok(Self {
            _lib: lib,
            bot_ptr,
            bot_tick_fn,
            bot_free_fn,
        })
    }

    /// Ticks the bot within a watchdog thread. If it hangs or takes longer than the timeout, fails.
    pub fn tick(
        &self,
        tick: u32,
        is_bot_1: bool,
        timeout: Duration,
        // Caches containing the state matching the bot's perspective:
        creeps: Vec<screeps_arena::objects::Creep>,
        spawns: Vec<screeps_arena::objects::StructureSpawn>,
        towers: Vec<screeps_arena::objects::StructureTower>,
        extensions: Vec<screeps_arena::objects::StructureExtension>,
        ramparts: Vec<screeps_arena::objects::StructureRampart>,
        containers: Vec<screeps_arena::objects::StructureContainer>,
        roads: Vec<screeps_arena::objects::StructureRoad>,
        walls: Vec<screeps_arena::objects::StructureWall>,
        resources: Vec<screeps_arena::objects::Resource>,
        sources: Vec<screeps_arena::objects::Source>,
        flags: Vec<screeps_arena::objects::Flag>,
        score_collectors: Vec<screeps_arena::objects::ScoreCollector>,
        bonus_flags: Vec<screeps_arena::objects::BonusFlag>,
        area_effects: Vec<screeps_arena::objects::AreaEffect>,
        construction_sites: Vec<screeps_arena::objects::ConstructionSite>,
    ) -> Result<Vec<QueuedAction>> {
        let action_queue = Arc::new(Mutex::new(Vec::new()));
        let queue_clone = Arc::clone(&action_queue);
        let bot_ptr = SendPtr(self.bot_ptr);
        let tick_fn = Arc::new(self.bot_tick_fn.clone());

        // Package execution context
        let context = BotExecutionContext {
            tick,
            is_bot_1,
            action_queue: queue_clone,
            creep_cache: creeps,
            spawn_cache: spawns,
            tower_cache: towers,
            extension_cache: extensions,
            rampart_cache: ramparts,
            container_cache: containers,
            road_cache: roads,
            wall_cache: walls,
            resource_cache: resources,
            source_cache: sources,
            flag_cache: flags,
            score_collector_cache: score_collectors,
            bonus_flag_cache: bonus_flags,
            area_effect_cache: area_effects,
            construction_site_cache: construction_sites,
        };

        // Run bot inside worker thread to enforce CPU limit watchdog
        let handle = thread::spawn(move || {
            let local_ptr = bot_ptr;
            CURRENT_BOT_CONTEXT.with(|ctx| {
                *ctx.borrow_mut() = Some(context);
            });

            unsafe {
                tick_fn(local_ptr.0);
            }

            CURRENT_BOT_CONTEXT.with(|ctx| {
                *ctx.borrow_mut() = None;
            });
        });

        // Simple watchdog implementation
        let start = std::time::Instant::now();
        while !handle.is_finished() {
            if start.elapsed() > timeout {
                return Err(anyhow::anyhow!(
                    "Execution timeout: Bot exceeded the CPU limit of {:?}",
                    timeout
                ));
            }
            thread::sleep(Duration::from_millis(1));
        }

        // Propagate thread execution failures/panics
        handle.join().map_err(|_| anyhow::anyhow!("Bot panicked during execution"))?;

        // Extract collected actions
        let collected = action_queue.lock().unwrap().clone();
        Ok(collected)
    }
}

impl Drop for BotDriver {
    fn drop(&mut self) {
        unsafe {
            (self.bot_free_fn)(self.bot_ptr);
        }
    }
}
