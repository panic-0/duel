//! 技能业务：技能定义与玩家的技能集合数据。
//! 技能集合是数据实例（owner 随玩家销毁）；正常行动由 Turn 流程主动查询。

use super::super::{
    operation::{Operation, OperationResult},
    state::BattleState,
    PlayerId,
};
use super::attack::AttackOperation;

/// 技能定义：按当前状态产生对应的 Operation，不另建执行器。
pub trait Ability: std::fmt::Debug {
    fn operation(&self, source_id: PlayerId, world: &BattleState) -> Option<Box<dyn Operation>>;
}

/// 连击技能：一次正常行动中连续发动指定次数的普攻。
#[derive(Debug, Clone, Copy)]
pub struct Combo {
    pub hits: u8,
}

impl Combo {
    pub fn new(hits: u8) -> Self {
        Self { hits }
    }
}

impl Ability for Combo {
    fn operation(&self, source_id: PlayerId, _world: &BattleState) -> Option<Box<dyn Operation>> {
        (self.hits > 0).then(|| {
            Box::new(ComboOperation {
                source_id,
                hits: self.hits,
            }) as Box<dyn Operation>
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ComboOperation {
    pub source_id: PlayerId,
    pub hits: u8,
}

impl Operation for ComboOperation {
    fn execute(
        self: Box<Self>,
        context: &mut super::super::operation::ActionContext<'_>,
    ) -> Result<super::super::operation::OperationOutcome, super::super::operation::OperationError>
    {
        let mut completed_hits = 0u8;
        for _ in 0..self.hits {
            let (result, _) = context.execute(AttackOperation::new(self.source_id))?;
            if result != OperationResult::Completed {
                break;
            }
            completed_hits += 1;
        }
        super::super::operation::completed_with(completed_hits)
    }
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
        world: &BattleState,
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
