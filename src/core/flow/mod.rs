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

/// 流程驱动：整个结算批次完成且未终局时，按事件分发顺序读取最终状态。
/// 返回值是后续流程事件；即时反应应由命令入队，不能依赖流程驱动结算。
///
/// 外部通过 BeforeTurn 等入口发起行动，不应直接加入 Turn。
/// Buff 响应 BeforeTurn(B) 产生的事件 E 属于当前批次：E 及其死亡、复活等
/// 反应全部结算后，才调用 advance。此时检查 B 是否存活，就能决定生成
/// Turn(B) 还是选择下一名玩家，无需提前生成行动再处理其失效。
pub trait FlowDriver: std::fmt::Debug {
    fn advance(&mut self, state: &GameState, event: &Event) -> Vec<Event>;
}

/// 结束条件：整条反应链结算后，依据最终状态判断是否结束。
/// 只接收本批次根事件，按条件装配顺序取首个命中结果。
pub trait EndCondition: std::fmt::Debug {
    fn check(&mut self, state: &GameState, event: &Event) -> Option<GameResult>;
}
