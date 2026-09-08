pub mod revival;
pub use revival::Revival;

pub mod damage_reduction;
pub use damage_reduction::DamageReduction;

use super::{
    ability::AbilitySet,
    command::Commands,
    event::{Event, EventType},
    operation::DamageContext,
    operation::{
        completed, DeathOperation, ExecutionContext, Operation, OperationError, OperationOutcome,
    },
    state::GameState,
    BuffId,
};

#[derive(Debug)]
pub(crate) struct DefaultDeathRule;

impl Buff for DefaultDeathRule {
    fn subscriptions(&self) -> Vec<(EventType, Priority)> {
        vec![(EventType::HpChanged, Priority::Final)]
    }

    fn operations(
        &mut self,
        event: &Event,
        world: &GameState,
        _buff_id: BuffId,
    ) -> Vec<Box<dyn Operation>> {
        match *event {
            Event::HpChanged { target_id, .. }
                if world
                    .get_player(target_id)
                    .is_some_and(|player| player.hp() == 0) =>
            {
                vec![Box::new(DeathOperation {
                    player_id: target_id,
                })]
            }
            _ => Vec::new(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct DefaultVictoryRule;

#[derive(Debug)]
struct EndLastStanding;

impl Operation for EndLastStanding {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        if context.state().get_players().len() <= 1 {
            context.end_game(super::flow::GameResult::LastStanding);
        }
        completed()
    }
}

impl Buff for DefaultVictoryRule {
    fn subscriptions(&self) -> Vec<(EventType, Priority)> {
        vec![(EventType::Checkpoint, Priority::Final)]
    }

    fn operations(
        &mut self,
        event: &Event,
        _world: &GameState,
        _buff_id: BuffId,
    ) -> Vec<Box<dyn Operation>> {
        if matches!(event, Event::Checkpoint { .. }) {
            vec![Box::new(EndLastStanding)]
        } else {
            Vec::new()
        }
    }
}

/// 事件分发优先级：变体声明顺序即触发顺序（小者先触发）。
/// 同优先级按 buff_id 升序（先注册先触发）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    /// 修改事件参数（如减伤改写伤害值），在其他监听者之前触发
    Modify,
    /// 常规监听
    Default,
    /// 结算（消费已被修改过的事件参数）
    Resolve,
    /// 兜底阶段，晚于结算触发（引擎的流程推进在所有 buff 之后进行）
    Final,
}

pub trait Buff: std::fmt::Debug {
    /// 角色归属；None 表示全局规则。
    fn owner(&self) -> Option<super::PlayerId> {
        None
    }

    /// 声明参与伤害参数修改及统一排序用的优先级。
    /// 与事件订阅分开注册：修改窗口由 Damage 操作开放，不绑定攻击事件。
    fn damage_modification(&self) -> Option<Priority> {
        None
    }

    /// 修改本次伤害草稿。只调整参数、声明消耗，不修改自身状态或世界；
    /// 反击、治疗等副作用应放到事件响应的操作里。
    fn modify_damage(&self, _context: &mut DamageContext, _world: &GameState, _buff_id: BuffId) {}

    fn operations(
        &mut self,
        _event: &Event,
        _world: &GameState,
        _buff_id: BuffId,
    ) -> Vec<Box<dyn Operation>> {
        Vec::new()
    }
    /// 声明该 buff 订阅的事件及触发优先级
    fn subscriptions(&self) -> Vec<(EventType, Priority)> {
        Vec::new()
    }

    /// 若该 buff 承载一名玩家的技能集合，返回查询接口；
    /// 正常行动由 Turn 流程主动向其逐个查询，而不是订阅 Turn 事件驱动。
    fn ability_set(&mut self) -> Option<&mut dyn AbilitySet> {
        None
    }

    /// 迁移适配：仅在旧 apply_event/queue_event 入口被调用；
    /// Operation 路径的事件是不可变事实，响应请通过 [`Self::operations`]。
    fn on_event(
        &mut self,
        _event: &mut Event,
        _world: &GameState,
        _commands: &mut Commands,
        _buff_id: BuffId,
    ) {
    }
}
