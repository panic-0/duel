//! 伤害业务：参数上下文、Damage 操作与修改规则注册表。
//! 减伤公式、护盾含义都在业务层；基础层只提供中性的关联提交。

use std::any::Any;
use std::fmt;

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

/// 一次伤害的参数草稿。修改规则只调整这里的数据、声明消耗与声明关联更新；
/// 提交由执行器的受控关联提交统一处理。
pub struct DamageContext {
    pub source_id: Option<PlayerId>,
    pub target_id: PlayerId,
    pub amount: u64,
    consumption: Vec<BuffId>,
    updates: Vec<(BuffId, Box<dyn Any>)>,
}

impl fmt::Debug for DamageContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DamageContext")
            .field("source_id", &self.source_id)
            .field("target_id", &self.target_id)
            .field("amount", &self.amount)
            .field("consumption", &self.consumption)
            .field("updates", &self.updates.len())
            .finish()
    }
}

impl DamageContext {
    pub fn new(source_id: Option<PlayerId>, target_id: PlayerId, amount: u64) -> Self {
        Self {
            source_id,
            target_id,
            amount,
            consumption: Vec::new(),
            updates: Vec::new(),
        }
    }

    pub fn reduce_to(&mut self, amount: u64) {
        self.amount = amount.min(self.amount);
    }

    /// 声明本次提交成功时消耗一次机会；执行器在提交后、通知前统一处理。
    pub fn consume_buff(&mut self, buff_id: BuffId) {
        self.consumption.push(buff_id);
    }

    /// 声明本次提交成功时就地更新一份数据实例（保留身份，不产生销毁事实）。
    /// 多次更新同一实例按声明顺序应用，最后一次生效。
    pub fn update_buff(&mut self, buff_id: BuffId, data: Box<dyn Any>) {
        self.updates.push((buff_id, data));
    }

    pub(crate) fn take_consumption(&mut self) -> Vec<BuffId> {
        std::mem::take(&mut self.consumption)
    }

    pub(crate) fn take_updates(&mut self) -> Vec<(BuffId, Box<dyn Any>)> {
        std::mem::take(&mut self.updates)
    }
}

/// 伤害修改规则：读取业务数据、修正伤害草稿、声明消耗与声明关联更新。
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
    /// 独立候选的主体顺序，来自与数据实例创建相同的共同单调顺序源，
    /// 可与实例的创建顺序直接比较。
    order: usize,
    rule: Box<dyn DamageRule>,
}

/// 业务规则注册表：经中性扩展存储持有，基础层不认识本类型。
/// entries 的下标即规则注册顺序。
#[derive(Debug, Default)]
pub struct DamageRules {
    entries: Vec<DamageRuleEntry>,
}

impl DamageRules {
    fn insert(&mut self, priority: Priority, order: usize, rule: Box<dyn DamageRule>) {
        self.entries.push(DamageRuleEntry {
            priority,
            order,
            rule,
        });
    }
}

/// 注册一条伤害修改规则。同一规则注册一次即可，
/// 它会在每次伤害的参数窗口中按统一候选顺序为适用的实例生效。
/// 独立候选的主体顺序从世界的共同顺序源取得，不使用注册表下标冒充。
pub fn register_damage_rule(
    world: &mut World,
    priority: Priority,
    rule: impl DamageRule + 'static,
) {
    let order = world.next_subject_order();
    let registry = world.resource_mut_or_insert_with(DamageRules::default);
    registry.insert(priority, order, Box::new(rule));
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
        // 写入前验证：初始目标不存在则伤害不成立，
        // 不得进入参数窗口，更不得提交以伤害成功为前提的资源消耗。
        if context.state().get_player(self.context.target_id).is_none() {
            return skipped();
        }
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
                            Subject::Standalone => entry.order,
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

        // 参数修改（含重定向等改写目标的方式）结束后：
        // 最终目标仍须存在，伤害才成立；不成立时连同声明的消耗与更新一并放弃。
        if context.state().get_player(self.context.target_id).is_none() {
            return skipped();
        }

        let amount = self.context.amount;
        let mut changes = ChangeSet::new().damage(self.context.target_id, amount);
        for id in self.context.take_consumption() {
            changes = changes.destroy(id, DestructionReason::Consumed);
        }
        for (id, data) in self.context.take_updates() {
            changes = changes.update_data(id, data);
        }
        let result = context.submit(changes)?;
        let Some(change) = result.hp else {
            // 目标不存在：按跳过处理，不伪造成功伤害。
            return skipped();
        };
        completed_with(change)
    }
}
