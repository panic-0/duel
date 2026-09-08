//! 技能业务：技能定义与玩家的技能集合数据。
//! 技能集合是数据实例（owner 随玩家销毁）；正常行动由 Turn 流程主动查询。

use super::super::{operation::Operation, state::GameState, PlayerId};

/// 技能定义：按当前状态产生对应的 Operation，不另建执行器。
pub trait Ability: std::fmt::Debug {
    fn operation(&self, source_id: PlayerId, world: &GameState) -> Option<Box<dyn Operation>>;
}

/// 一名玩家的技能集合数据。Turn 流程按当前状态逐个查询正常行动，
/// 每次查询都重新判断资格；`usize` 是集合内稳定的技能槽位（注册顺序），
/// 不随可用性过滤而漂移，避免列表变化导致漏执行或重复。
#[derive(Debug)]
pub struct Abilities {
    /// 业务上的技能持有者；记录 owner 只表示生命周期依赖。
    holder: PlayerId,
    abilities: Vec<Box<dyn Ability>>,
}

impl Abilities {
    pub fn new(holder: PlayerId, abilities: Vec<Box<dyn Ability>>) -> Self {
        Abilities { holder, abilities }
    }

    pub fn holder(&self) -> PlayerId {
        self.holder
    }

    /// 按当前状态收集此刻可用的正常行动。
    pub fn normal_actions(
        &self,
        source_id: PlayerId,
        world: &GameState,
    ) -> Vec<(usize, Box<dyn Operation>)> {
        self.abilities
            .iter()
            .enumerate()
            .filter_map(|(slot, ability)| {
                ability
                    .operation(source_id, world)
                    .map(|operation| (slot, operation))
            })
            .collect()
    }
}
