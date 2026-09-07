use super::{
    log::{LogEntry, Logger},
    player::Player,
    PlayerId,
};
use std::collections::BTreeMap;

/// 游戏状态：玩家数据与日志。不含 buff、事件队列等引擎机制，
/// 监听者（buff/ability/流程驱动/结束条件）只拿到本结构的只读引用。
#[derive(Debug)]
pub struct GameState {
    players: BTreeMap<PlayerId, Player>,
    player_id_counter: PlayerId,
    logger: Logger,
}

impl GameState {
    pub fn new() -> Self {
        GameState {
            players: BTreeMap::new(),
            player_id_counter: 0,
            logger: Logger::default(),
        }
    }

    pub fn add_player(&mut self, player: Player) -> PlayerId {
        let id = self.player_id_counter;
        self.player_id_counter += 1;
        self.players.insert(id, player);
        id
    }

    pub fn remove_player(&mut self, id: PlayerId) -> Option<Player> {
        self.players.remove(&id)
    }

    pub fn get_players(&self) -> &BTreeMap<PlayerId, Player> {
        &self.players
    }

    pub fn get_player(&self, id: PlayerId) -> Option<&Player> {
        self.players.get(&id)
    }

    pub fn get_player_mut(&mut self, id: PlayerId) -> Option<&mut Player> {
        self.players.get_mut(&id)
    }

    /// id 之后第一个存活的玩家（不回绕）
    pub fn get_next_player_not_around(&self, id: PlayerId) -> Option<PlayerId> {
        self.players.range(id + 1..).next().map(|(id, _)| *id)
    }

    /// id 之后第一个存活的玩家（回绕到队首）
    pub fn get_next_player_around(&self, id: PlayerId) -> Option<PlayerId> {
        self.players
            .range(id + 1..)
            .next()
            .or_else(|| self.players.range(..id).next())
            .map(|(id, _)| *id)
    }

    pub fn set_logger(&mut self, logger: Logger) {
        self.logger = logger;
    }

    pub fn log(&self, entry: LogEntry) {
        self.logger.emit(self, &entry);
    }
}

impl Default for GameState {
    fn default() -> Self {
        GameState::new()
    }
}
