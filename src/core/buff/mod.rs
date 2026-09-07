pub mod revival;
pub use revival::Revival;

pub mod damage_reduction;
pub use damage_reduction::DamageReduction;

use super::{
    command::Commands,
    event::{Event, EventType},
    world::World,
};

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
    StateMachine,
}

pub trait Buff: std::fmt::Debug {
    /// 声明该 buff 订阅的事件及触发优先级
    fn subscriptions(&self) -> Vec<(EventType, Priority)>;
    fn on_event(
        &self,
        event: &mut Event,
        world: &World,
        commands: &mut Commands,
        buff_id: super::BuffId,
    );
}
