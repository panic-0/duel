//! 伤害业务：参数上下文、Damage 操作与修改规则注册表。
//! 减伤公式、护盾含义都在业务层；基础层只提供中性的关联提交。

use super::super::{
    buff_data::DestructionReason,
    operation::{
        completed_with, skipped, ChangeSet, ExecutionContext, Operation, OperationError,
        OperationOutcome,
    },
    query::Query,
    system::{Priority, Subject},
    world::World,
    BuffId, PlayerId,
};

/// 一次伤害的参数草稿。修改规则只调整这里的数据并声明消耗；
/// 提交与消耗由执行器的受控关联提交统一处理。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DamageContext {
    pub source_id: Option<PlayerId>,
    pub target_id: PlayerId,
    pub amount: u64,
    consumption: Vec<BuffId>,
}

impl DamageContext {
    pub fn new(source_id: Option<PlayerId>, target_id: PlayerId, amount: u64) -> Self {
        Self {
            source_id,
            target_id,
            amount,
            consumption: Vec::new(),
        }
    }

    pub fn reduce_to(&mut self, amount: u64) {
        self.amount = amount.min(self.amount);
    }

    /// 声明本次提交成功时消耗一次机会；执行器在提交后、通知前统一处理。
    pub fn consume_buff(&mut self, buff_id: BuffId) {
        self.consumption.push(buff_id);
    }

    pub(crate) fn take_consumption(&mut self) -> Vec<BuffId> {
        std::mem::take(&mut self.consumption)
    }
}

/// 伤害修改规则：读取业务数据、修正伤害草稿并声明消耗。
/// 只调整参数，不执行子 Operation，不直接修改世界或 Buff 数据；
/// 也不得批量处理所有实例而绕过候选级排序。
pub trait DamageRule: std::fmt::Debug {
    /// 收集本规则对本次伤害草稿适用的候选主体。
    fn candidates(&self, context: &DamageContext, query: &Query<'_>) -> Vec<Subject>;

    /// 修改一个候选对应的伤害参数。
    fn modify(&self, context: &mut DamageContext, subject: Subject, query: &Query<'_>);
}

#[derive(Debug)]
struct DamageRuleEntry {
    priority: Priority,
    rule: Box<dyn DamageRule>,
}

/// 业务规则注册表：经中性扩展存储持有，基础层不认识本类型。
/// entries 的下标即规则注册顺序。
#[derive(Debug, Default)]
pub struct DamageRules {
    entries: Vec<DamageRuleEntry>,
}

impl DamageRules {
    fn insert(&mut self, priority: Priority, rule: Box<dyn DamageRule>) {
        self.entries.push(DamageRuleEntry { priority, rule });
    }
}

/// 注册一条伤害修改规则。同一规则注册一次即可，
/// 它会在每次伤害的参数窗口中按统一候选顺序为适用的实例生效。
pub fn register_damage_rule(
    world: &mut World,
    priority: Priority,
    rule: impl DamageRule + 'static,
) {
    let registry = world.resource_mut_or_insert_with(DamageRules::default);
    registry.insert(priority, Box::new(rule));
}

#[derive(Debug)]
pub struct Damage {
    pub context: DamageContext,
}

impl Damage {
    pub fn new(source_id: Option<PlayerId>, target_id: PlayerId, amount: u64) -> Self {
        Self {
            context: DamageContext::new(source_id, target_id, amount),
        }
    }
}

impl Operation for Damage {
    fn execute(
        mut self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        // 参数修改窗口：候选 =（Priority, 响应主体稳定顺序, 规则注册顺序），
        // 前一个修改对后一个可见。
        let registry = context.resource::<DamageRules>();
        let mut candidates: Vec<(Priority, usize, usize, Subject)> = Vec::new();
        {
            let query = context.query();
            if let Some(registry) = registry {
                for (rule_order, entry) in registry.entries.iter().enumerate() {
                    for subject in entry.rule.candidates(&self.context, &query) {
                        let subject_order = match subject {
                            Subject::Instance(id) => query.subject_order(id).unwrap_or(0),
                            Subject::Standalone => rule_order,
                        };
                        candidates.push((entry.priority, subject_order, rule_order, subject));
                    }
                }
            }
        }
        candidates.sort();
        {
            let query = context.query();
            for (_, _, rule_order, subject) in &candidates {
                if let Some(registry) = registry {
                    registry.entries[*rule_order]
                        .rule
                        .modify(&mut self.context, *subject, &query);
                }
            }
        }

        let amount = self.context.amount;
        let mut changes = ChangeSet::new().damage(self.context.target_id, amount);
        for id in self.context.take_consumption() {
            changes = changes.destroy(id, DestructionReason::Consumed);
        }
        let result = context.submit(changes)?;
        let Some(change) = result.hp else {
            // 目标不存在：按跳过处理，不伪造成功伤害。
            return skipped();
        };
        completed_with(change)
    }
}
