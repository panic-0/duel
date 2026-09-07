pub mod end_conditions;
pub mod round_robin;

pub use end_conditions::{LastManStanding, RoundLimit};
pub use round_robin::RoundRobinFlow;

use super::{event::Event, state::GameState};

/// 引擎内置的默认回合上限，防止无限对局挂起
pub const MAX_ROUNDS: u32 = 10000;

/// 对局结束的判定结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameResult {
    /// 剩最后一名玩家
    LastStanding,
    /// 达到回合上限，平局
    Draw,
}

/// 流程驱动：定义对局的回合/轮转结构，在每个事件处理完毕后生成后续流程事件
pub trait FlowDriver: std::fmt::Debug {
    fn advance(&mut self, state: &GameState, event: &Event) -> Vec<Event>;
}

/// 结束条件：在每个事件分发前检查，命中则对局结束且该事件不再分发给监听者
pub trait EndCondition: std::fmt::Debug {
    fn check(&mut self, state: &GameState, event: &Event) -> Option<GameResult>;
}
