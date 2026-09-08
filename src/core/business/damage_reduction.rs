//! 减伤业务：按业务目标字段匹配的减伤数据与规则。
//! `owner` 只表示生命周期依赖；是否生效由规则在响应时按**当前**
//! 伤害上下文核对目标字段决定——同一窗口内允许改写目标（如重定向），
//! 候选收集因此不做目标排除，只收集全部现存实例。

use super::super::{
    engine::BattleEngine,
    log::LogEntry,
    query::Query,
    system::{Priority, ReactionTarget},
    ComponentId, PlayerId,
};
use super::damage::{register_damage_rule, DamageDraft, DamageRule};

/// 减伤数据实例。`target_id` 是业务作用范围；`owner` 只决定随谁销毁。
#[derive(Debug)]
pub struct DamageReductionData {
    pub target_id: PlayerId,
    pub reduction_ratio: f64,
}

impl DamageReductionData {
    pub fn new(target_id: PlayerId, reduction_ratio: f64) -> Self {
        Self {
            target_id,
            reduction_ratio,
        }
    }
}

/// 减伤规则：候选收集包含全部现存实例（身份只收集一次，符合候选快照规则）；
/// 是否真正生效由 modify 按当时的伤害上下文核对——先行的重定向规则
/// 改写目标后，减伤应跟随实际受伤者，而不是收集时的旧目标。
#[derive(Debug, Default)]
pub struct DamageReductionRule;

impl DamageRule for DamageReductionRule {
    fn candidates(&self, _context: &DamageDraft, query: &Query<'_>) -> Vec<ReactionTarget> {
        query
            .components::<DamageReductionData>()
            .iter()
            .map(|(id, _, _)| ReactionTarget::Instance(*id))
            .collect()
    }

    fn modify(&self, context: &mut DamageDraft, subject: ReactionTarget, query: &Query<'_>) {
        let ReactionTarget::Instance(id) = subject else {
            return;
        };
        let Some(data) = query.component::<DamageReductionData>(id) else {
            return;
        };
        // 按当前目标核对作用范围：可变匹配条件不在收集阶段冻结。
        if data.target_id != context.target_id {
            return;
        }
        let original = context.amount;
        let reduced = (original as f64 * (1.0 - data.reduction_ratio)) as u64;
        context.reduce_to(reduced);
        let target_id = data.target_id;
        query.state().log(LogEntry::DamageReduced {
            target_id,
            original,
            reduced,
        });
    }
}

/// 注册减伤规则（一次即可），之后每份数据实例自动参与伤害参数窗口。
pub fn register_damage_reduction_rule(world: &mut BattleEngine) {
    register_damage_rule(world, Priority::Modify, DamageReductionRule);
}

/// 注册一份数据实例，返回身份。`owner` 是生命周期依赖，`target_id` 是作用范围，
/// 二者可以指向不同角色。
pub fn attach_damage_reduction(
    world: &mut BattleEngine,
    owner: PlayerId,
    target_id: PlayerId,
    reduction_ratio: f64,
) -> ComponentId {
    world.attach_component(
        Some(owner),
        DamageReductionData {
            target_id,
            reduction_ratio,
        },
    )
}
