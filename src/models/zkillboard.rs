use serde::{Deserialize, Serialize};

// pub mod zkillboard {
#[derive(Serialize, Deserialize)]
pub struct ZKillmail {
    attackers: Vec<KillAttacker>,
    killmail_id: i32,
    killmail_time: String,
    solar_system_id: i32,
    victim: KillVictim,
    zkb: ZKillStats,
}

#[derive(Serialize, Deserialize)]
struct KillAttacker {
    alliance_id: Option<i32>,
    character_id: i32,
    corporation_id: i32,
    damage_done: Option<i32>,
    final_blow: bool,
    security_status: f64,
    ship_type_id: i32,
    weapon_type_id: i32,
}

#[derive(Serialize, Deserialize)]
struct KillVictim {
    alliance_id: i32,
    character_id: i32,
    corporation_id: i32,
    damage_taken : i32,
    items: Vec<KillmailItem>,
    position: ZKillLocation,
    ship_type_id: i32,
}

#[derive(Serialize, Deserialize)]
struct KillmailItem {
    flag: i32,
    item_type_id: i32,
    quantity_destroyed: Option<i32>,
    quantity_dropped: Option<i32>,
    singleton: i32,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ZKillStats {
    location_id: i32,
    hash: String,
    fitted_value: f64,
    dropped_value: f64,
    destroyed_value: f64,
    total_value: f64,
    points: i32,
    npc: bool,
    solo: bool,
    awox: bool,
    esi: String,
    url: String,
}

#[derive(Serialize, Deserialize)]
struct ZKillLocation {
    x: f64,
    y: f64,
    z: f64,
}
// }