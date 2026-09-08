//! 治疗业务：治疗操作与生命修正操作，都是对中性生命变化的业务包装。

use super::super::{
    operation::{
        completed_with, skipped, ChangeSet, ExecutionContext, Operation, OperationError,
        OperationOutcome, OperationResult,
    },
    PlayerId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Heal {
    pub target_id: PlayerId,
    pub amount: u64,
}

impl Heal {
    pub fn new(target_id: PlayerId, amount: u64) -> Self {
        Self { target_id, amount }
    }
}

impl Operation for Heal {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        let changes = ChangeSet::new().heal(self.target_id, self.amount);
        let result = context.submit(changes)?;
        let Some(change) = result.hp else {
            return skipped();
        };
        completed_with(change)
    }
}

/// 带符号的生命修正操作（正数治疗、负数伤害）。
#[derive(Debug, Clone, Copy)]
pub struct HpModifier {
    target_id: PlayerId,
    modifier: i64,
}

impl HpModifier {
    pub fn new(target_id: PlayerId, modifier: i64) -> Self {
        Self {
            target_id,
            modifier,
        }
    }

    pub fn damage(target_id: PlayerId, amount: u64) -> Self {
        Self::new(target_id, -(amount.min(i64::MAX as u64) as i64))
    }

    pub fn heal(target_id: PlayerId, amount: u64) -> Self {
        Self::new(target_id, amount.min(i64::MAX as u64) as i64)
    }
}

impl Operation for HpModifier {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        let Some(change) = context.modify_hp(self.target_id, self.modifier)? else {
            return Ok((OperationResult::Skipped, None));
        };
        Ok((OperationResult::Completed, Some(Box::new(change))))
    }
}
