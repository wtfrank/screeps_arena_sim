use anyhow::{Context, Result};
use colored::*;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct DiffOptions {
    pub target_tick: Option<u32>,
    pub verbose: bool,
    pub ignore_users: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffSeverity {
    MissingObject,
    ExtraObject,
    FieldMismatch,
    TickCountMismatch,
    MetadataMismatch,
}

#[derive(Debug, Clone)]
pub struct DiffDetail {
    pub tick: u32,
    pub object_id: Option<String>,
    pub field: String,
    pub val1: String,
    pub val2: String,
    pub severity: DiffSeverity,
}

pub struct ReplayDiffTool {
    options: DiffOptions,
}

impl ReplayDiffTool {
    pub fn new(options: DiffOptions) -> Self {
        Self { options }
    }

    pub fn compare_files<P: AsRef<Path>>(&self, path1: P, path2: P) -> Result<Vec<DiffDetail>> {
        let content1 = fs::read_to_string(&path1)
            .with_context(|| format!("Failed to read replay file: {}", path1.as_ref().display()))?;
        let content2 = fs::read_to_string(&path2)
            .with_context(|| format!("Failed to read replay file: {}", path2.as_ref().display()))?;

        let ticks1: Vec<Value> = serde_json::from_str(&content1)
            .with_context(|| format!("Failed to parse JSON replay: {}", path1.as_ref().display()))?;
        let ticks2: Vec<Value> = serde_json::from_str(&content2)
            .with_context(|| format!("Failed to parse JSON replay: {}", path2.as_ref().display()))?;

        Ok(self.compare_ticks(&ticks1, &ticks2))
    }

    pub fn compare_ticks(&self, ticks1: &[Value], ticks2: &[Value]) -> Vec<DiffDetail> {
        let mut diffs = Vec::new();

        // Discover and build user ID normalization mapping across both replays
        let user_map = build_user_mapping(ticks1, ticks2);

        if ticks1.len() != ticks2.len() {
            diffs.push(DiffDetail {
                tick: 0,
                object_id: None,
                field: "tick_count".to_string(),
                val1: ticks1.len().to_string(),
                val2: ticks2.len().to_string(),
                severity: DiffSeverity::TickCountMismatch,
            });
        }

        let min_ticks = ticks1.len().min(ticks2.len());

        for i in 0..min_ticks {
            let t1 = &ticks1[i];
            let t2 = &ticks2[i];

            let tick_num = t1.get("gameTime").and_then(|v| v.as_u64()).unwrap_or((i + 1) as u64) as u32;

            if let Some(target) = self.options.target_tick {
                if tick_num != target {
                    continue;
                }
            }

            // Compare top level gameTime
            let tick_num2 = t2.get("gameTime").and_then(|v| v.as_u64()).unwrap_or((i + 1) as u64) as u32;
            if tick_num != tick_num2 {
                diffs.push(DiffDetail {
                    tick: tick_num,
                    object_id: None,
                    field: "gameTime".to_string(),
                    val1: tick_num.to_string(),
                    val2: tick_num2.to_string(),
                    severity: DiffSeverity::MetadataMismatch,
                });
            }

            // Compare objects array in tick
            let objs1_arr = t1.get("objects").and_then(|v| v.as_array());
            let objs2_arr = t2.get("objects").and_then(|v| v.as_array());

            match (objs1_arr, objs2_arr) {
                (Some(o1), Some(o2)) => {
                    self.compare_objects_list(tick_num, o1, o2, &user_map, &mut diffs);
                }
                (Some(_), None) => {
                    diffs.push(DiffDetail {
                        tick: tick_num,
                        object_id: None,
                        field: "objects".to_string(),
                        val1: "Array".to_string(),
                        val2: "Missing".to_string(),
                        severity: DiffSeverity::MetadataMismatch,
                    });
                }
                (None, Some(_)) => {
                    diffs.push(DiffDetail {
                        tick: tick_num,
                        object_id: None,
                        field: "objects".to_string(),
                        val1: "Missing".to_string(),
                        val2: "Array".to_string(),
                        severity: DiffSeverity::MetadataMismatch,
                    });
                }
                _ => {}
            }
        }

        diffs
    }

    fn compare_objects_list(
        &self,
        tick: u32,
        objs1: &[Value],
        objs2: &[Value],
        user_map: &HashMap<String, String>,
        diffs: &mut Vec<DiffDetail>,
    ) {
        let mut map1: HashMap<String, &Value> = HashMap::new();
        let mut map2: HashMap<String, &Value> = HashMap::new();

        for obj in objs1 {
            if let Some(id_str) = extract_object_id(obj) {
                map1.insert(id_str, obj);
            }
        }
        for obj in objs2 {
            if let Some(id_str) = extract_object_id(obj) {
                map2.insert(id_str, obj);
            }
        }

        let keys1: HashSet<_> = map1.keys().cloned().collect();
        let keys2: HashSet<_> = map2.keys().cloned().collect();

        // Check missing objects in replay2
        for id in keys1.difference(&keys2) {
            diffs.push(DiffDetail {
                tick,
                object_id: Some(id.clone()),
                field: "object_presence".to_string(),
                val1: format_obj_summary(map1.get(id).copied()),
                val2: "Missing".to_string(),
                severity: DiffSeverity::MissingObject,
            });
        }

        // Check extra objects in replay2
        for id in keys2.difference(&keys1) {
            diffs.push(DiffDetail {
                tick,
                object_id: Some(id.clone()),
                field: "object_presence".to_string(),
                val1: "Missing".to_string(),
                val2: format_obj_summary(map2.get(id).copied()),
                severity: DiffSeverity::ExtraObject,
            });
        }

        // Compare matching objects
        for id in keys1.intersection(&keys2) {
            let o1 = map1.get(id).unwrap();
            let o2 = map2.get(id).unwrap();
            self.compare_single_object(tick, id, o1, o2, user_map, diffs);
        }
    }

    fn compare_single_object(
        &self,
        tick: u32,
        id: &str,
        o1: &Value,
        o2: &Value,
        user_map: &HashMap<String, String>,
        diffs: &mut Vec<DiffDetail>,
    ) {
        let obj1_map = match o1.as_object() {
            Some(m) => m,
            None => return,
        };
        let obj2_map = match o2.as_object() {
            Some(m) => m,
            None => return,
        };

        let keys1: HashSet<_> = obj1_map.keys().cloned().collect();
        let keys2: HashSet<_> = obj2_map.keys().cloned().collect();

        // Compare fields present in both or missing
        let all_keys: HashSet<_> = keys1.union(&keys2).cloned().collect();
        let mut sorted_keys: Vec<_> = all_keys.into_iter().collect();
        sorted_keys.sort();

        for key in sorted_keys {
            let val1 = obj1_map.get(&key);
            let val2 = obj2_map.get(&key);

            match (val1, val2) {
                (Some(v1), Some(v2)) => {
                    let norm1 = normalize_user_field(&key, v1, user_map);
                    let norm2 = normalize_user_field(&key, v2, user_map);

                    if !compare_json_values(&norm1, &norm2) {
                        diffs.push(DiffDetail {
                            tick,
                            object_id: Some(id.to_string()),
                            field: key.clone(),
                            val1: norm1.to_string(),
                            val2: norm2.to_string(),
                            severity: DiffSeverity::FieldMismatch,
                        });
                    }
                }
                (Some(v1), None) => {
                    let norm1 = normalize_user_field(&key, v1, user_map);
                    diffs.push(DiffDetail {
                        tick,
                        object_id: Some(id.to_string()),
                        field: key.clone(),
                        val1: norm1.to_string(),
                        val2: "Missing".to_string(),
                        severity: DiffSeverity::FieldMismatch,
                    });
                }
                (None, Some(v2)) => {
                    let norm2 = normalize_user_field(&key, v2, user_map);
                    diffs.push(DiffDetail {
                        tick,
                        object_id: Some(id.to_string()),
                        field: key.clone(),
                        val1: "Missing".to_string(),
                        val2: norm2.to_string(),
                        severity: DiffSeverity::FieldMismatch,
                    });
                }
                (None, None) => {}
            }
        }
    }
}

/// Discovers users in both replays and maps player usernames/IDs to normalized tokens (e.g. `userA`, `userB`)
fn build_user_mapping(ticks1: &[Value], ticks2: &[Value]) -> HashMap<String, String> {
    let mut map = HashMap::new();

    let users1 = extract_users(ticks1);
    let users2 = extract_users(ticks2);

    for (i, uid1) in users1.iter().enumerate() {
        let label = format!("user_{}", (b'A' + i as u8) as char);
        map.insert(uid1.clone(), label.clone());
        if i < users2.len() {
            map.insert(users2[i].clone(), label);
        }
    }
    for (i, uid2) in users2.iter().enumerate() {
        if i >= users1.len() {
            let label = format!("user_{}", (b'A' + i as u8) as char);
            map.insert(uid2.clone(), label);
        }
    }

    map
}

fn extract_users(ticks: &[Value]) -> Vec<String> {
    let mut users = Vec::new();
    if let Some(t1) = ticks.first() {
        if let Some(u_obj) = t1.get("users").and_then(|v| v.as_object()) {
            let mut keys: Vec<_> = u_obj.keys().cloned().collect();
            keys.sort();
            for k in keys {
                users.push(k);
            }
        }
        if users.is_empty() {
            if let Some(objs) = t1.get("objects").and_then(|v| v.as_array()) {
                let mut seen = HashSet::new();
                for obj in objs {
                    if let Some(u) = obj.get("user").and_then(|v| v.as_str()) {
                        if seen.insert(u.to_string()) {
                            users.push(u.to_string());
                        }
                    }
                }
            }
        }
    }
    users
}

fn normalize_user_field(key: &str, val: &Value, user_map: &HashMap<String, String>) -> Value {
    if key == "user" || key == "controlledBy" {
        if let Some(s) = val.as_str() {
            if let Some(norm) = user_map.get(s) {
                return Value::String(norm.clone());
            }
        }
    }
    val.clone()
}

fn extract_object_id(obj: &Value) -> Option<String> {
    obj.get("_id").map(|v| match v {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => v.to_string(),
    })
}

fn format_obj_summary(obj: Option<&Value>) -> String {
    match obj {
        Some(o) => {
            let proto = o.get("prototypeName").and_then(|v| v.as_str()).unwrap_or("Object");
            let x = o.get("x").and_then(|v| v.as_i64()).unwrap_or(-1);
            let y = o.get("y").and_then(|v| v.as_i64()).unwrap_or(-1);
            format!("{} at ({}, {})", proto, x, y)
        }
        None => "Missing".to_string(),
    }
}

fn compare_json_values(v1: &Value, v2: &Value) -> bool {
    if v1 == v2 {
        return true;
    }

    match (v1, v2) {
        (Value::Array(a1), Value::Array(a2)) => {
            if a1.len() != a2.len() {
                return false;
            }
            for (elem1, elem2) in a1.iter().zip(a2.iter()) {
                if !compare_json_values(elem1, elem2) {
                    return false;
                }
            }
            true
        }
        (Value::Object(m1), Value::Object(m2)) => {
            if m1.len() != m2.len() {
                return false;
            }
            for (k, val1) in m1 {
                match m2.get(k) {
                    Some(val2) => {
                        if !compare_json_values(val1, val2) {
                            return false;
                        }
                    }
                    None => return false,
                }
            }
            true
        }
        _ => false,
    }
}

pub fn print_diff_report(diffs: &[DiffDetail], verbose: bool) {
    if diffs.is_empty() {
        println!("{}", "✓ Replays match perfectly! No differences found.".green().bold());
        return;
    }

    println!("{}", format!("❌ Found {} differences between replay files:", diffs.len()).red().bold());
    println!("{}", "================================================================================".yellow());

    let mut by_tick: HashMap<u32, Vec<&DiffDetail>> = HashMap::new();
    for d in diffs {
        by_tick.entry(d.tick).or_default().push(d);
    }

    let mut ticks: Vec<_> = by_tick.keys().cloned().collect();
    ticks.sort();

    for t in ticks {
        let tick_diffs = by_tick.get(&t).unwrap();
        println!("{} [Tick {}] - {} discrepancy(ies):", "▶".blue(), t, tick_diffs.len());

        for d in tick_diffs {
            let id_str = d.object_id.as_deref().unwrap_or("GLOBAL");
            if verbose {
                println!(
                    "   • [{}] Field '{}':\n       Replay 1: {}\n       Replay 2: {}",
                    id_str.cyan(),
                    d.field.yellow(),
                    d.val1.red(),
                    d.val2.green()
                );
            } else {
                println!(
                    "   • [{}] {}: {} vs {}",
                    id_str.cyan(),
                    d.field.yellow(),
                    d.val1.red(),
                    d.val2.green()
                );
            }
        }
    }
    println!("{}", "================================================================================".yellow());
}
