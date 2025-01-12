use serde::{Serialize, Serializer};
use serde_json::Map;
use std::sync::Arc;
use steamid_ng::SteamID;

use crate::{
    io::{g15::G15Player, regexes::StatusLine},
    player_records::{PlayerRecord, Verdict},
};

// Player

#[derive(Debug, Clone, Serialize)]
pub struct Player {
    pub name: Arc<str>,
    #[serde(rename = "steamID64", serialize_with = "serialize_steamid_as_string")]
    pub steamid: SteamID,
    #[serde(rename = "isSelf")]
    pub is_self: bool,
    #[serde(rename = "gameInfo")]
    pub game_info: GameInfo,
    #[serde(rename = "steamInfo")]
    pub steam_info: Option<SteamInfo>,
    #[serde(rename = "customData")]
    pub custom_data: serde_json::Value,
    pub tags: Vec<Arc<str>>,
    #[serde(rename = "localVerdict")]
    pub local_verdict: Verdict,
    pub convicted: bool,
    #[serde(rename = "previousNames")]
    pub previous_names: Vec<Arc<str>>,
}

impl Player {
    pub(crate) fn new_from_status(status: &StatusLine, user: Option<SteamID>) -> Player {
        let is_self = user.map(|user| user == status.steamid).unwrap_or(false);
        Player {
            name: status.name.clone(),
            steamid: status.steamid,
            is_self,
            game_info: GameInfo::new_from_status(status),
            steam_info: None,
            custom_data: serde_json::Value::Object(Map::new()),
            tags: Vec::new(),
            local_verdict: Verdict::Player,
            convicted: false,
            previous_names: Vec::new(),
        }
    }

    pub(crate) fn new_from_g15(g15: &G15Player, user: Option<SteamID>) -> Option<Player> {
        let steamid = g15.steamid?;
        let game_info = GameInfo::new_from_g15(g15)?;
        let is_self = user.map(|user| user == steamid).unwrap_or(false);

        Some(Player {
            name: g15.name.clone()?,
            steamid,
            is_self,
            game_info,
            steam_info: None,
            custom_data: serde_json::Value::Object(Map::new()),
            tags: Vec::new(),
            local_verdict: Verdict::Player,
            convicted: false,
            previous_names: Vec::new(),
        })
    }

    /// Given a record, update this player with the included data.
    pub fn update_from_record(&mut self, record: PlayerRecord) {
        if record.steamid != self.steamid {
            tracing::error!("Updating player with wrong record.");
            return;
        }

        self.custom_data = record.custom_data;
        self.local_verdict = record.verdict;
        self.previous_names = record.previous_names;
    }

    /// Create a record from the current player
    #[allow(dead_code)]
    pub fn get_record(&self) -> PlayerRecord {
        PlayerRecord {
            steamid: self.steamid,
            custom_data: self.custom_data.clone(),
            verdict: self.local_verdict,
            previous_names: self.previous_names.clone(),
        }
    }
}

// PlayerState

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
pub enum PlayerState {
    Active,
    Spawning,
    Disconnected,
}

// Team

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Team {
    Unassigned = 0,
    Spectators = 1,
    Red = 2,
    Blu = 3,
}

impl TryFrom<u32> for Team {
    type Error = &'static str;
    fn try_from(val: u32) -> Result<Self, Self::Error> {
        match val {
            0 => Ok(Team::Unassigned),
            1 => Ok(Team::Spectators),
            2 => Ok(Team::Red),
            3 => Ok(Team::Blu),
            _ => Err("Not a valid team value"),
        }
    }
}

impl Serialize for Team {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        s.serialize_u32(*self as u32)
    }
}

// SteamInfo

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamInfo {
    #[serde(rename = "name")]
    pub account_name: Arc<str>,
    pub profile_url: Arc<str>,
    #[serde(rename = "pfp")]
    pub pfp_url: Arc<str>,
    pub pfp_hash: Arc<str>,
    pub profile_visibility: ProfileVisibility,
    pub time_created: Option<i64>,
    pub country_code: Option<Arc<str>>,

    pub vac_bans: i64,
    pub game_bans: i64,
    pub days_since_last_ban: Option<i64>,
    // pub friends: Vec<Friend>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Friend {
    #[serde(rename = "steamID64")]
    pub steamid: SteamID,
    #[serde(rename = "friendSince")]
    pub friend_since: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ProfileVisibility {
    Private = 1,
    FriendsOnly = 2,
    Public = 3,
}

impl From<i32> for ProfileVisibility {
    fn from(value: i32) -> Self {
        match value {
            1 => ProfileVisibility::Private,
            2 => ProfileVisibility::FriendsOnly,
            3 => ProfileVisibility::Public,
            _ => ProfileVisibility::Private,
        }
    }
}

// GameInfo

#[derive(Debug, Clone, Serialize)]
pub struct GameInfo {
    pub userid: Arc<str>,
    pub team: Team,
    pub time: u32,
    pub ping: u32,
    pub loss: u32,
    pub state: PlayerState,
    pub kills: u32,
    pub deaths: u32,
    #[serde(skip)]
    /// How many cycles has passed since the player has been seen
    last_seen: u32,
}

impl GameInfo {
    pub(crate) fn new_from_g15(g15: &G15Player) -> Option<GameInfo> {
        Some(GameInfo {
            userid: g15.userid.clone()?,
            team: g15.team.unwrap_or(Team::Unassigned),
            time: 0,
            ping: g15.ping.unwrap_or(0),
            loss: 0,
            state: PlayerState::Active,
            kills: g15.score.unwrap_or(0),
            deaths: g15.deaths.unwrap_or(0),
            last_seen: 0,
        })
    }

    pub(crate) fn new_from_status(status: &StatusLine) -> GameInfo {
        GameInfo {
            userid: status.userid.clone(),
            team: Team::Unassigned,
            time: status.time,
            ping: status.ping,
            loss: status.loss,
            state: status.state,
            kills: 0,
            deaths: 0,
            last_seen: 0,
        }
    }

    pub(crate) fn next_cycle(&mut self) {
        const DISCONNECTED_THRESHOLD: u32 = 1;

        self.last_seen += 1;
        if self.last_seen > DISCONNECTED_THRESHOLD {
            self.state = PlayerState::Disconnected;
        }
    }

    pub(crate) fn acknowledge(&mut self) {
        self.last_seen = 0;

        if self.state == PlayerState::Disconnected {
            self.state = PlayerState::Spawning;
        }
    }

    pub(crate) fn should_prune(&self) -> bool {
        const CYCLE_LIMIT: u32 = 5;
        self.last_seen > CYCLE_LIMIT
    }
}

// Useful

fn serialize_steamid_as_string<S: Serializer>(steamid: &SteamID, s: S) -> Result<S::Ok, S::Error> {
    format!("{}", u64::from(*steamid)).serialize(s)
}
