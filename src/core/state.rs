//! 玩家与结构化日志组成的战斗状态。运行期修改由 BattleEngine 统一提交。

use super::{
    log::{LogEntry, Logger},
    player::Player,
    PlayerId,
};
use std::collections::BTreeMap;

/// 游戏状态：玩家数据与日志。不含 buff、事件队列等引擎机制，
/// 监听者（buff/ability/流程驱动/结束条件）只拿到本结构的只读引用。
#[derive(Debug)]
pub struct BattleState {
    players: BTreeMap<PlayerId, Player>,
    player_id_counter: PlayerId,
    logger: Logger,
}

impl BattleState {
    pub fn new() -> Self {
        BattleState {
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

    pub fn players(&self) -> &BTreeMap<PlayerId, Player> {
        &self.players
    }

    pub fn player(&self, id: PlayerId) -> Option<&Player> {
        self.players.get(&id)
    }

    pub(crate) fn get_player_mut(&mut self, id: PlayerId) -> Option<&mut Player> {
        self.players.get_mut(&id)
    }

    /// id 之后第一个存活玩家；到队尾后回绕到队首。
    pub fn get_next_player_around(&self, id: PlayerId) -> Option<PlayerId> {
        self.players
            .range(id + 1..)
            .find(|(_, player)| player.is_alive())
            .or_else(|| {
                self.players
                    .range(..id)
                    .find(|(_, player)| player.is_alive())
            })
            .map(|(id, _)| *id)
    }

    pub fn set_logger(&mut self, logger: Logger) {
        self.logger = logger;
    }

    pub fn log(&self, entry: LogEntry) {
        self.logger.emit(self, &entry);
    }
}

impl Default for BattleState {
    fn default() -> Self {
        BattleState::new()
    }
}
