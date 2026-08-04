use anyhow::{Context, Result};
use libloading::{Library, Symbol};
use std::os::raw::{c_char, c_void};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use screeps_arena::ffi::{ActionId, HostInterface, PrototypeId};

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
    pub owned_structure_cache: Vec<screeps_arena::objects::OwnedStructure>,
    pub terrain_cache: Vec<Vec<crate::models::Terrain>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueuedAction {
    pub actor_id: String,
    pub action: ActionId,
    pub target_id: Option<String>,
    pub arg1: usize,
    pub arg2: usize,
}

/// Direct FFI callbacks routed from the bot back to the simulator host
extern "C" fn get_ticks_callback() -> u32 {
    CURRENT_BOT_CONTEXT.with(|ctx| ctx.borrow().as_ref().map(|c| c.tick).unwrap_or(0))
}

extern "C" fn get_cpu_time_callback() -> u32 {
    // Return dummy CPU elapsed time (or track actual thread CPU execution time)
    0
}

extern "C" fn get_terrain_at_callback(x: u8, y: u8) -> u32 {
    CURRENT_BOT_CONTEXT.with(|ctx| {
        if let Some(ref c) = *ctx.borrow() {
            let xu = x as usize;
            let yu = y as usize;
            if xu < c.terrain_cache.len() && yu < c.terrain_cache[xu].len() {
                match c.terrain_cache[xu][yu] {
                    crate::models::Terrain::Wall => 1,
                    crate::models::Terrain::Swamp => 2,
                    crate::models::Terrain::Plain => 0,
                }
            } else {
                0
            }
        } else {
            0
        }
    })
}

extern "C" fn get_objects_callback(
    proto_id: u32,
    out_ptr: *mut *const c_void,
    out_len: *mut usize,
) {
    CURRENT_BOT_CONTEXT.with(|ctx| {
        if let Some(ref c) = *ctx.borrow() {
            unsafe {
                match proto_id {
                    1 => {
                        // Creep
                        *out_ptr = c.creep_cache.as_ptr() as *const c_void;
                        *out_len = c.creep_cache.len();
                    }
                    2 => {
                        // StructureSpawn
                        *out_ptr = c.spawn_cache.as_ptr() as *const c_void;
                        *out_len = c.spawn_cache.len();
                    }
                    3 => {
                        // StructureTower
                        *out_ptr = c.tower_cache.as_ptr() as *const c_void;
                        *out_len = c.tower_cache.len();
                    }
                    4 => {
                        // StructureExtension
                        *out_ptr = c.extension_cache.as_ptr() as *const c_void;
                        *out_len = c.extension_cache.len();
                    }
                    5 => {
                        // StructureRampart
                        *out_ptr = c.rampart_cache.as_ptr() as *const c_void;
                        *out_len = c.rampart_cache.len();
                    }
                    6 => {
                        // StructureContainer
                        *out_ptr = c.container_cache.as_ptr() as *const c_void;
                        *out_len = c.container_cache.len();
                    }
                    7 => {
                        // StructureRoad
                        *out_ptr = c.road_cache.as_ptr() as *const c_void;
                        *out_len = c.road_cache.len();
                    }
                    8 => {
                        // StructureWall
                        *out_ptr = c.wall_cache.as_ptr() as *const c_void;
                        *out_len = c.wall_cache.len();
                    }
                    9 => {
                        // Resource
                        *out_ptr = c.resource_cache.as_ptr() as *const c_void;
                        *out_len = c.resource_cache.len();
                    }
                    10 => {
                        // Source
                        *out_ptr = c.source_cache.as_ptr() as *const c_void;
                        *out_len = c.source_cache.len();
                    }
                    11 => {
                        // Flag
                        *out_ptr = c.flag_cache.as_ptr() as *const c_void;
                        *out_len = c.flag_cache.len();
                    }
                    12 => {
                        // ScoreCollector
                        *out_ptr = c.score_collector_cache.as_ptr() as *const c_void;
                        *out_len = c.score_collector_cache.len();
                    }
                    13 => {
                        // BonusFlag
                        *out_ptr = c.bonus_flag_cache.as_ptr() as *const c_void;
                        *out_len = c.bonus_flag_cache.len();
                    }
                    14 => {
                        // AreaEffect
                        *out_ptr = c.area_effect_cache.as_ptr() as *const c_void;
                        *out_len = c.area_effect_cache.len();
                    }
                    15 => {
                        // ConstructionSite
                        *out_ptr = c.construction_site_cache.as_ptr() as *const c_void;
                        *out_len = c.construction_site_cache.len();
                    }
                    16 => {
                        // OwnedStructure
                        *out_ptr = c.owned_structure_cache.as_ptr() as *const c_void;
                        *out_len = c.owned_structure_cache.len();
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
        let actor = std::ffi::CStr::from_ptr(actor_id)
            .to_string_lossy()
            .into_owned();
        let target = if target_id.is_null() {
            None
        } else {
            Some(
                std::ffi::CStr::from_ptr(target_id)
                    .to_string_lossy()
                    .into_owned(),
            )
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

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct BotTickMessage {
    pub tick: u32,
    pub is_bot_1: bool,
    pub creeps: Vec<screeps_arena::objects::Creep>,
    pub spawns: Vec<screeps_arena::objects::StructureSpawn>,
    pub towers: Vec<screeps_arena::objects::StructureTower>,
    pub extensions: Vec<screeps_arena::objects::StructureExtension>,
    pub ramparts: Vec<screeps_arena::objects::StructureRampart>,
    pub containers: Vec<screeps_arena::objects::StructureContainer>,
    pub roads: Vec<screeps_arena::objects::StructureRoad>,
    pub walls: Vec<screeps_arena::objects::StructureWall>,
    pub resources: Vec<screeps_arena::objects::Resource>,
    pub sources: Vec<screeps_arena::objects::Source>,
    pub flags: Vec<screeps_arena::objects::Flag>,
    pub score_collectors: Vec<screeps_arena::objects::ScoreCollector>,
    pub bonus_flags: Vec<screeps_arena::objects::BonusFlag>,
    pub area_effects: Vec<screeps_arena::objects::AreaEffect>,
    pub construction_sites: Vec<screeps_arena::objects::ConstructionSite>,
    pub owned_structures: Vec<screeps_arena::objects::OwnedStructure>,
    pub terrain: Vec<Vec<crate::models::Terrain>>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum HostToWorkerMessage {
    Tick(BotTickMessage),
    Shutdown,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum WorkerToHostMessage {
    TickResult(Result<Vec<QueuedAction>, String>),
    Initialized(Result<(), String>),
}

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};

pub struct BotDriver {
    child: Child,
    stream: UnixStream,
}

impl BotDriver {
    pub fn load(path: &Path, bot_label: &str, enable_debug: bool) -> Result<Self> {
        let (host_stream, worker_stream) =
            UnixStream::pair().context("Failed to create UnixSocket pair")?;

        let exe_path = std::env::current_exe().context("Failed to get current executable path")?;
        let worker_fd = worker_stream.into_raw_fd();

        // Ensure worker_fd remains open across exec
        unsafe {
            let flags = libc::fcntl(worker_fd, libc::F_GETFD);
            if flags != -1 {
                libc::fcntl(worker_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
            }
        }

        let mut command = if enable_debug {
            let port = 12345;
            let mut cmd = Command::new("gdbserver");
            cmd.arg(format!(":{}", port))
                .arg(exe_path)
                .arg("bot-runner")
                .arg(path.to_str().unwrap())
                .arg(worker_fd.to_string());
            cmd
        } else {
            let mut cmd = Command::new(exe_path);
            cmd.arg("bot-runner")
                .arg(path.to_str().unwrap())
                .arg(worker_fd.to_string());
            cmd
        };

        if let Ok(rust_log) = std::env::var("RUST_LOG") {
            command.env("RUST_LOG", rust_log);
        }

        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = command
            .spawn()
            .context("Failed to spawn bot-runner worker process")?;

        // Read stdout and stderr asynchronously in background threads with log prefixing
        if let Some(stdout) = child.stdout.take() {
            let label = bot_label.to_string();
            thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines().flatten() {
                    println!("[{}] {}", label, line);
                }
            });
        }

        if let Some(stderr) = child.stderr.take() {
            let label = bot_label.to_string();
            thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().flatten() {
                    eprintln!("[{}] {}", label, line);
                }
            });
        }

        if enable_debug {
            // Give gdbserver a brief moment to print "Listening on port..."
            thread::sleep(Duration::from_millis(50));
            println!(
                "\ngdb -ex \"target remote :12345\" -ex \"break wtfbot::Bot::tick\" -ex \"continue\""
            );
        }

        // Wait for initialization acknowledgment from worker with child process crash monitoring
        let init_msg = loop {
            host_stream.set_read_timeout(Some(Duration::from_millis(200)))?;
            match bincode_read::<WorkerToHostMessage, _>(&host_stream) {
                Ok(msg) => break msg,
                Err(e) => {
                    if let Ok(Some(status)) = child.try_wait() {
                        return Err(anyhow::anyhow!(
                            "Bot worker process exited unexpectedly during init with status: {}",
                            status
                        ));
                    }
                    if let Some(io_err) = e.downcast_ref::<std::io::Error>() {
                        if io_err.kind() == std::io::ErrorKind::WouldBlock
                            || io_err.kind() == std::io::ErrorKind::TimedOut
                        {
                            continue;
                        }
                    }
                    return Err(anyhow::anyhow!("IPC read error during init: {:?}", e));
                }
            }
        };
        match init_msg {
            WorkerToHostMessage::Initialized(Ok(())) => {}
            WorkerToHostMessage::Initialized(Err(err)) => {
                let _ = child.kill();
                return Err(anyhow::anyhow!("Bot initialization error: {}", err));
            }
            _ => {
                let _ = child.kill();
                return Err(anyhow::anyhow!("Unexpected worker initialization response"));
            }
        }

        Ok(Self {
            child,
            stream: host_stream,
        })
    }

    pub fn tick(&mut self, msg: BotTickMessage, timeout: Duration) -> Result<Vec<QueuedAction>> {
        bincode_write(&self.stream, &HostToWorkerMessage::Tick(msg))?;

        let start = std::time::Instant::now();
        let response = loop {
            self.stream
                .set_read_timeout(Some(Duration::from_millis(100)))?;
            match bincode_read::<WorkerToHostMessage, _>(&self.stream) {
                Ok(res) => break res,
                Err(e) => {
                    if let Ok(Some(status)) = self.child.try_wait() {
                        return Err(anyhow::anyhow!(
                            "Worker process exited with status: {}",
                            status
                        ));
                    }
                    if start.elapsed() > timeout {
                        let _ = self.child.kill();
                        return Err(anyhow::anyhow!(
                            "Worker process timeout after {:?}",
                            timeout
                        ));
                    }
                    if let Some(io_err) = e.downcast_ref::<std::io::Error>() {
                        if io_err.kind() == std::io::ErrorKind::WouldBlock
                            || io_err.kind() == std::io::ErrorKind::TimedOut
                        {
                            continue;
                        }
                    }
                    let _ = self.child.kill();
                    return Err(anyhow::anyhow!("Worker process IPC error: {:?}", e));
                }
            }
        };

        match response {
            WorkerToHostMessage::TickResult(Ok(actions)) => Ok(actions),
            WorkerToHostMessage::TickResult(Err(err_msg)) => {
                Err(anyhow::anyhow!("Bot panic: {}", err_msg))
            }
            _ => Err(anyhow::anyhow!("Invalid response from worker process")),
        }
    }
}

impl Drop for BotDriver {
    fn drop(&mut self) {
        let _ = bincode_write(&self.stream, &HostToWorkerMessage::Shutdown);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn bincode_write<T: serde::Serialize, W: Write>(mut writer: W, val: &T) -> Result<()> {
    let bytes = serde_json::to_vec(val)?;
    let len = bytes.len() as u32;
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

fn bincode_read<T: serde::de::DeserializeOwned, R: Read>(mut reader: R) -> Result<T> {
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes)?;
    let len = u32::from_be_bytes(len_bytes) as usize;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    let val = serde_json::from_slice(&buf)?;
    Ok(val)
}

/// Worker mode entrypoint executed by child processes (`screeps_arena_sim bot-runner <bot_path> <fd> [--pause]`)
pub fn run_bot_runner_process(bot_path: &str, socket_fd: i32, pause: bool) -> Result<()> {
    let _ = env_logger::try_init();
    let stream = unsafe { UnixStream::from_raw_fd(socket_fd) };

    let lib = unsafe { Library::new(bot_path).context("Failed to load bot library in worker")? };

    let set_host_interface_fn: Symbol<unsafe extern "C" fn(HostInterface)> = unsafe {
        lib.get(b"set_host_interface")
            .context("Failed to bind set_host_interface")?
    };
    let bot_initialize_fn: Symbol<unsafe extern "C" fn() -> *mut c_void> = unsafe {
        lib.get(b"bot_initialize")
            .context("Failed to bind bot_initialize")?
    };
    let bot_tick_fn: Symbol<unsafe extern "C" fn(*mut c_void)> =
        unsafe { lib.get(b"bot_tick").context("Failed to bind bot_tick")? };

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

    let init_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        bot_initialize_fn()
    }));

    let bot_ptr = match init_res {
        Ok(ptr) if !ptr.is_null() => {
            bincode_write(&stream, &WorkerToHostMessage::Initialized(Ok(())))?;
            ptr
        }
        Ok(_) => {
            bincode_write(
                &stream,
                &WorkerToHostMessage::Initialized(Err("bot_initialize returned null".to_string())),
            )?;
            return Ok(());
        }
        Err(err) => {
            let msg = panic_err_to_string(err);
            bincode_write(&stream, &WorkerToHostMessage::Initialized(Err(msg)))?;
            return Ok(());
        }
    };

    loop {
        let msg: HostToWorkerMessage = match bincode_read(&stream) {
            Ok(m) => m,
            Err(_) => break, // Host closed socket or exited
        };

        match msg {
            HostToWorkerMessage::Shutdown => break,
            HostToWorkerMessage::Tick(tick_msg) => {
                let action_queue = Arc::new(Mutex::new(Vec::new()));
                let context = BotExecutionContext {
                    tick: tick_msg.tick,
                    is_bot_1: tick_msg.is_bot_1,
                    action_queue: action_queue.clone(),
                    creep_cache: tick_msg.creeps,
                    spawn_cache: tick_msg.spawns,
                    tower_cache: tick_msg.towers,
                    extension_cache: tick_msg.extensions,
                    rampart_cache: tick_msg.ramparts,
                    container_cache: tick_msg.containers,
                    road_cache: tick_msg.roads,
                    wall_cache: tick_msg.walls,
                    resource_cache: tick_msg.resources,
                    source_cache: tick_msg.sources,
                    flag_cache: tick_msg.flags,
                    score_collector_cache: tick_msg.score_collectors,
                    bonus_flag_cache: tick_msg.bonus_flags,
                    area_effect_cache: tick_msg.area_effects,
                    construction_site_cache: tick_msg.construction_sites,
                    owned_structure_cache: tick_msg.owned_structures,
                    terrain_cache: tick_msg.terrain,
                };

                CURRENT_BOT_CONTEXT.with(|ctx| {
                    *ctx.borrow_mut() = Some(context);
                });

                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
                    bot_tick_fn(bot_ptr);
                }));

                CURRENT_BOT_CONTEXT.with(|ctx| {
                    *ctx.borrow_mut() = None;
                });

                match res {
                    Ok(()) => {
                        let actions = action_queue.lock().unwrap().clone();
                        bincode_write(&stream, &WorkerToHostMessage::TickResult(Ok(actions)))?;
                    }
                    Err(err) => {
                        let msg = panic_err_to_string(err);
                        bincode_write(&stream, &WorkerToHostMessage::TickResult(Err(msg)))?;
                    }
                }
            }
        }
    }

    Ok(())
}

fn panic_err_to_string(err: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = err.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = err.downcast_ref::<String>() {
        s.clone()
    } else {
        "Panic occurred".to_string()
    }
}
