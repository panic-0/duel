//! 减伤业务：按业务目标字段匹配的减伤数据与规则。
//! `owner` 只表示生命周期依赖；是否生效由规则的 target 匹配决定。

use super::super::{
    log::LogEntry,
    query::Query,
    system::{Priority, Subject},
    world::World,
    BuffId, PlayerId,
};
use super::damage::{register_damage_rule, DamageContext, DamageRule};

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

/// 减伤规则：为每个适用的数据实例产生一个候选。
#[derive(Debug, Default)]
pub struct DamageReductionRule;

impl DamageRule for DamageReductionRule {
    fn candidates(&self, context: &DamageContext, query: &Query<'_>) -> Vec<Subject> {
        query
            .instances::<DamageReductionData>()
            .iter()
            .filter(|(_, _, data)| data.target_id == context.target_id)
            .map(|(id, _, _)| Subject::Instance(*id))
            .collect()
    }

    fn modify(&self, context: &mut DamageContext, subject: Subject, query: &Query<'_>) {
        let Subject::Instance(id) = subject else {
            return;
        };
        let Some(data) = query.data::<DamageReductionData>(id) else {
            return;
        };
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
pub fn register_damage_reduction_rule(world: &mut World) {
    register_damage_rule(world, Priority::Modify, DamageReductionRule);
}

/// 注册一份数据实例，返回身份。`owner` 是生命周期依赖，`target_id` 是作用范围，
/// 二者可以指向不同角色。
pub fn add_damage_reduction(
    world: &mut World,
    owner: PlayerId,
    target_id: PlayerId,
    reduction_ratio: f64,
) -> BuffId {
    world.add_data(
        Some(owner),
        DamageReductionData {
            target_id,
            reduction_ratio,
        },
    )
}
