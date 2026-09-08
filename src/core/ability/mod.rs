pub mod attack;
pub use attack::{Attack, AttackOperation};

use super::{buff::Buff, operation::Operation, state::GameState, PlayerId};

/// 一名玩家的技能集合。Turn 流程按当前状态逐个查询正常行动，
/// 每次查询都重新判断资格，已不适用或已执行的技能不再重复产生。
pub trait AbilitySet: std::fmt::Debug {
    /// 按当前状态收集此刻可用的正常行动。
    /// `usize` 是集合内稳定的技能槽位（注册顺序），不随可用性过滤而漂移；
    /// 流程以“已执行的最远槽位”为游标继续查询，避免列表变化导致漏执行或重复。
    fn normal_actions(
        &mut self,
        source_id: PlayerId,
        world: &GameState,
    ) -> Vec<(usize, Box<dyn Operation>)>;
}

pub trait Ability: std::fmt::Debug {
    fn operation(&self, source_id: PlayerId, world: &GameState) -> Option<Box<dyn Operation>>;
}

/// 技能集合的默认承载。仍以 Buff 注册以获得角色归属与死亡清理，
/// 但不订阅事件驱动行动——正常行动由 Turn 流程主动查询。
#[derive(Debug)]
pub struct Abilities {
    source_id: PlayerId,
    abilities: Vec<Box<dyn Ability>>,
}

impl Abilities {
    pub fn new(source_id: PlayerId, abilities: Vec<Box<dyn Ability>>) -> Self {
        Abilities {
            source_id,
            abilities,
        }
    }
}

impl AbilitySet for Abilities {
    fn normal_actions(
        &mut self,
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

impl Buff for Abilities {
    fn owner(&self) -> Option<PlayerId> {
        Some(self.source_id)
    }

    fn ability_set(&mut self) -> Option<&mut dyn AbilitySet> {
        Some(self)
    }
}
